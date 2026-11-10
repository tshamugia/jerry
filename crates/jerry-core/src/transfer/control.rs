//! Pausing, cancelling and answering a running job from another thread.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use super::conflict::Answer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Running,
    Paused,
    Cancelled,
}

/// What a job should do at a checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Flow {
    Go,
    /// Go on, after a pause: the time paused doesn't count towards the speed.
    Resumed,
    Cancelled,
}

/// Pause, resume, cancel and the answer to a conflict for one job. The job looks at it between
/// files and between a file's chunks, so a pause holds the file being copied open until it
/// resumes. A job that meets a conflict waits here for [`answer`](Self::answer).
#[derive(Debug)]
pub struct Control {
    inner: Mutex<Inner>,
    changed: Condvar,
}

#[derive(Debug)]
struct Inner {
    state: State,
    /// The answer to the conflict the job reported, until it takes it.
    answer: Option<Answer>,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner {
                state: State::Running,
                answer: None,
            }),
            changed: Condvar::new(),
        }
    }
}

impl Control {
    pub fn new() -> Self {
        Self::default()
    }

    /// Holds the job at its next checkpoint. Nothing changes once it is cancelled.
    pub fn pause(&self) {
        let mut inner = self.lock();
        if inner.state == State::Running {
            inner.state = State::Paused;
        }
    }

    /// Lets a paused job go on. A cancelled job stays cancelled.
    pub fn resume(&self) {
        let mut inner = self.lock();
        if inner.state == State::Paused {
            inner.state = State::Running;
            self.changed.notify_all();
        }
    }

    /// Ends the job at its next checkpoint, paused or not, or while it waits for an answer. It
    /// can't be undone.
    pub fn cancel(&self) {
        self.lock().state = State::Cancelled;
        self.changed.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.lock().state == State::Cancelled
    }

    /// Answers the conflict the job reported ([`Event::Conflict`](super::Event::Conflict)), and
    /// the job goes on. One given while the job reports it is there when the job asks.
    pub fn answer(&self, answer: Answer) {
        self.lock().answer = Some(answer);
        self.changed.notify_all();
    }

    /// Forgets an answer given before the question it would answer.
    pub(super) fn forget_answer(&self) {
        self.lock().answer = None;
    }

    /// Blocks until the conflict the job reported is answered. `None` if the job is cancelled
    /// meanwhile. A pause doesn't count here: it holds the job at its next checkpoint.
    pub(super) fn wait_answer(&self) -> Option<Answer> {
        let mut inner = self.lock();
        loop {
            if inner.state == State::Cancelled {
                return None;
            }
            if let Some(answer) = inner.answer.take() {
                return Some(answer);
            }
            inner = self
                .changed
                .wait(inner)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Blocks while the job is paused, and says what to do next.
    pub(super) fn checkpoint(&self) -> Flow {
        let mut inner = self.lock();
        let mut waited = false;
        while inner.state == State::Paused {
            waited = true;
            inner = self
                .changed
                .wait(inner)
                .unwrap_or_else(PoisonError::into_inner);
        }
        match (inner.state, waited) {
            (State::Cancelled, _) => Flow::Cancelled,
            (_, true) => Flow::Resumed,
            (_, false) => Flow::Go,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::transfer::conflict::Choice;

    #[test]
    fn running_jobs_go_on() {
        let control = Control::new();
        assert_eq!(control.checkpoint(), Flow::Go);
        control.resume();
        assert_eq!(control.checkpoint(), Flow::Go);
    }

    #[test]
    fn a_pause_holds_the_job_until_it_resumes() {
        let control = Arc::new(Control::new());
        control.pause();
        let (sender, receiver) = mpsc::channel();
        let worker = control.clone();
        let handle = std::thread::spawn(move || sender.send(worker.checkpoint()));
        assert!(
            receiver.recv_timeout(Duration::from_millis(50)).is_err(),
            "the job waits"
        );
        control.resume();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)),
            Ok(Flow::Resumed)
        );
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn cancelling_ends_a_pause_and_can_not_be_undone() {
        let control = Arc::new(Control::new());
        control.pause();
        let worker = control.clone();
        let handle = std::thread::spawn(move || worker.checkpoint());
        std::thread::sleep(Duration::from_millis(20));
        control.cancel();
        assert_eq!(handle.join().unwrap(), Flow::Cancelled);

        control.resume();
        control.pause();
        assert!(control.is_cancelled());
        assert_eq!(control.checkpoint(), Flow::Cancelled);
    }

    #[test]
    fn a_job_waits_for_its_answer_and_a_pause_does_not_hold_it() {
        let control = Arc::new(Control::new());
        control.pause();
        let worker = control.clone();
        let (sender, receiver) = mpsc::channel();
        let handle = std::thread::spawn(move || sender.send(worker.wait_answer()));
        assert!(
            receiver.recv_timeout(Duration::from_millis(50)).is_err(),
            "the job waits"
        );
        let answer = Answer {
            choice: Choice::KeepBoth,
            all: true,
        };
        control.answer(answer);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)),
            Ok(Some(answer))
        );
        handle.join().unwrap().unwrap();
        assert_eq!(control.lock().state, State::Paused, "still paused");
    }

    #[test]
    fn cancelling_ends_the_wait_for_an_answer() {
        let control = Arc::new(Control::new());
        let worker = control.clone();
        let handle = std::thread::spawn(move || worker.wait_answer());
        std::thread::sleep(Duration::from_millis(20));
        control.cancel();
        assert_eq!(handle.join().unwrap(), None);
    }

    #[test]
    fn an_answer_given_before_the_question_is_forgotten() {
        let control = Control::new();
        control.answer(Answer {
            choice: Choice::Skip,
            all: false,
        });
        control.forget_answer();
        control.cancel();
        assert_eq!(control.wait_answer(), None, "nothing left to take");
    }
}

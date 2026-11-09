//! Pausing and cancelling a running job from another thread.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

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

/// Pause, resume and cancel for one job. The job looks at it between files and between a file's
/// chunks, so a pause holds the file being copied open until it resumes.
#[derive(Debug)]
pub struct Control {
    state: Mutex<State>,
    changed: Condvar,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::Running),
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
        let mut state = self.lock();
        if *state == State::Running {
            *state = State::Paused;
        }
    }

    /// Lets a paused job go on. A cancelled job stays cancelled.
    pub fn resume(&self) {
        let mut state = self.lock();
        if *state == State::Paused {
            *state = State::Running;
            self.changed.notify_all();
        }
    }

    /// Ends the job at its next checkpoint, paused or not. It can't be undone.
    pub fn cancel(&self) {
        *self.lock() = State::Cancelled;
        self.changed.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        *self.lock() == State::Cancelled
    }

    /// Blocks while the job is paused, and says what to do next.
    pub(super) fn checkpoint(&self) -> Flow {
        let mut state = self.lock();
        let mut waited = false;
        while *state == State::Paused {
            waited = true;
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        match (*state, waited) {
            (State::Cancelled, _) => Flow::Cancelled,
            (_, true) => Flow::Resumed,
            (_, false) => Flow::Go,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

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
}

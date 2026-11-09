//! How fast a job is copying: bytes per second over the last few seconds.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How far back the speed looks.
const WINDOW: Duration = Duration::from_secs(3);
/// How often a sample is taken. Windows reports a fast disk's chunks far more often than this.
const SAMPLE_EVERY: Duration = Duration::from_millis(100);

/// The speed over a sliding window of samples of the bytes copied so far.
#[derive(Debug, Default)]
pub(super) struct Speed {
    samples: VecDeque<(Instant, u64)>,
    current: u64,
}

impl Speed {
    /// Takes in `copied`, the bytes copied so far (never less than last time), and returns the
    /// speed: 0 until two samples exist.
    pub(super) fn update(&mut self, now: Instant, copied: u64) -> u64 {
        let due = self
            .samples
            .back()
            .is_none_or(|&(at, _)| now.saturating_duration_since(at) >= SAMPLE_EVERY);
        if !due {
            return self.current;
        }
        self.samples.push_back((now, copied));
        while self.samples.len() > 2
            && self
                .samples
                .front()
                .is_some_and(|&(at, _)| now.saturating_duration_since(at) > WINDOW)
        {
            self.samples.pop_front();
        }
        if let Some(&(at, bytes)) = self.samples.front() {
            let seconds = now.saturating_duration_since(at).as_secs_f64();
            if seconds > 0.0 {
                self.current = (copied.saturating_sub(bytes) as f64 / seconds) as u64;
            }
        }
        self.current
    }

    /// Forgets the samples, after a pause: time spent paused isn't time spent copying.
    pub(super) fn restart(&mut self) {
        self.samples.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1_000_000;

    fn after(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn unknown_until_two_samples_exist() {
        let start = Instant::now();
        let mut speed = Speed::default();
        assert_eq!(speed.update(start, 0), 0);
        assert_eq!(speed.update(after(start, 10), MB), 0, "too soon to sample");
    }

    #[test]
    fn measures_bytes_per_second() {
        let start = Instant::now();
        let mut speed = Speed::default();
        speed.update(start, 0);
        assert_eq!(speed.update(after(start, 500), 50 * MB), 100 * MB);
    }

    #[test]
    fn looks_only_at_the_last_seconds() {
        let start = Instant::now();
        let mut speed = Speed::default();
        speed.update(start, 0);
        speed.update(after(start, 1000), MB);
        // Then ten times faster: the slow start falls out of the window.
        speed.update(after(start, 4000), 31 * MB);
        assert_eq!(speed.update(after(start, 5000), 41 * MB), 10 * MB);
    }

    #[test]
    fn a_pause_does_not_count() {
        let start = Instant::now();
        let mut speed = Speed::default();
        speed.update(start, 0);
        speed.update(after(start, 1000), 10 * MB);
        speed.restart();
        speed.update(after(start, 61_000), 10 * MB);
        assert_eq!(speed.update(after(start, 62_000), 20 * MB), 10 * MB);
    }
}

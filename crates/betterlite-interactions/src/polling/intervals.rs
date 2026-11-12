use std::time::Duration;

pub struct PollIntervals {
    pub interval: Duration,
    pub maximum: Duration,
    pub elapsed: Duration,
    pub timeout: Duration,
}

impl Iterator for PollIntervals {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        if self.interval.is_zero() {
            return None;
        }
        let elapsed = self.elapsed.checked_add(self.interval)?;
        if elapsed > self.timeout {
            return None;
        }
        let yielded = self.interval;
        self.elapsed = elapsed;
        self.interval = self.interval.saturating_mul(2).min(self.maximum);
        Some(yielded)
    }
}

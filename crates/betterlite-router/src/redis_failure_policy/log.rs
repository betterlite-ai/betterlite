use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogDecision {
    Debug,
    Level { suppressed: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeoutLogThrottle {
    pub interval: Duration,
    pub last_logged_at: Option<Duration>,
    pub suppressed: u32,
}

impl TimeoutLogThrottle {
    #[must_use]
    pub fn admit(&mut self, now: Duration) -> Option<u32> {
        if let Some(last) = self.last_logged_at
            && now.saturating_sub(last) < self.interval
        {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        let suppressed = self.suppressed;
        self.suppressed = 0;
        self.last_logged_at = Some(now);
        Some(suppressed)
    }
}

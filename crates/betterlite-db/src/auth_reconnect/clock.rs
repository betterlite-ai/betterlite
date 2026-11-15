use std::time::Instant;

pub trait ReconnectClock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl ReconnectClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

use chrono::{DateTime, Utc};

pub trait UtcClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl UtcClock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

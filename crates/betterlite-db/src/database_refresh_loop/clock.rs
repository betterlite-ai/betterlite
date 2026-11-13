use chrono::{DateTime, Utc};

pub trait RefreshClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl RefreshClock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

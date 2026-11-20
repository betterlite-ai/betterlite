use chrono::{DateTime, Utc};

pub trait InvalidationClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl InvalidationClock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

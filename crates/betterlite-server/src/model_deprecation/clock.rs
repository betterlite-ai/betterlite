use chrono::{DateTime, Utc};

pub trait UtcClock {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl UtcClock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

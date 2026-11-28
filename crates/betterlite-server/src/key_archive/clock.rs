use chrono::{DateTime, Utc};

pub trait ArchiveClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemArchiveClock;

impl ArchiveClock for SystemArchiveClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

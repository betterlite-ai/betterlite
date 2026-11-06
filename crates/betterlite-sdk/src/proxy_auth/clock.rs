use chrono::Utc;

pub trait Clock: Send + Sync {
    fn unix_seconds(&self) -> i64;
}

#[derive(Debug, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_seconds(&self) -> i64 {
        Utc::now().timestamp()
    }
}

use chrono::{DateTime, FixedOffset, Local};

// litellm's datetime.now(): the process time zone decides which local minute a counter rolls over in
pub trait LegacyClock: Send + Sync {
    fn now(&self) -> DateTime<FixedOffset>;
}

// the wall clock a running proxy reads
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemLegacyClock;

impl LegacyClock for SystemLegacyClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Local::now().fixed_offset()
    }
}

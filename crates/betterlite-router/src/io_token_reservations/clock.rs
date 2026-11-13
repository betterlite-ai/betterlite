use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub const ROUTING_ARGS_TTL_SECONDS: u64 = 60;

const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 3600;
const MINUTES_PER_HOUR: u64 = 60;
const HOURS_PER_DAY: u64 = 24;

pub type IoClock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

// LiteLLM formats get_utc_datetime as %H-%M, so the cache key follows UTC and not the host zone
#[must_use]
pub fn minute_label(now: SystemTime) -> String {
    let seconds = epoch_seconds(now);
    let hours = seconds.checked_div(SECONDS_PER_HOUR).unwrap_or_default();
    let hour = hours.checked_rem(HOURS_PER_DAY).unwrap_or_default();
    let minutes = seconds.checked_div(SECONDS_PER_MINUTE).unwrap_or_default();
    let minute = minutes.checked_rem(MINUTES_PER_HOUR).unwrap_or_default();
    format!("{hour:02}-{minute:02}")
}

#[must_use]
pub fn seconds_until_minute_reset(now: SystemTime) -> u64 {
    let seconds = epoch_seconds(now);
    let second = seconds.checked_rem(SECONDS_PER_MINUTE).unwrap_or_default();
    let remaining = SECONDS_PER_MINUTE.saturating_sub(second);
    remaining.max(1)
}

fn epoch_seconds(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

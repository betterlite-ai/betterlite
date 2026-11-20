use chrono::{DateTime, TimeDelta, Utc};
use rust_decimal::prelude::ToPrimitive as _;

const NANOS_PER_SECOND: f64 = 1_000_000_000.0;
const SECONDS_PER_HOUR: f64 = 3_600.0;

// LiteLLM's _get_remaining_hours_for_budget_reset, with the clock passed in instead of read from datetime.now()
#[must_use]
// ast-grep-ignore: no-long-names
pub fn remaining_hours_for_budget_reset(reset_at: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
    let remaining: TimeDelta = reset_at.signed_duration_since(now);
    let seconds = remaining.num_seconds().to_f64().unwrap_or_default();
    let fraction = f64::from(remaining.subsec_nanos()).div(NANOS_PER_SECOND);
    seconds.add(fraction).div(SECONDS_PER_HOUR)
}

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::otel::legacy_policy::span_time::{SpanTime, to_ns};

const NANOS_PER_SECOND: i64 = 1_000_000_000;

// the failure hooks truncate a float stamp to whole seconds before scaling, as LiteLLM's int(int(x) * 1e9) does
#[must_use]
pub(super) fn whole_second_ns(time: SpanTime, now: i64) -> i64 {
    match time {
        SpanTime::Missing => now,
        SpanTime::Seconds(seconds) => seconds_to_ns(seconds),
        SpanTime::DateTime(_) => to_ns(time, now),
    }
}

fn seconds_to_ns(seconds: f64) -> i64 {
    let whole = Decimal::from_f64_retain(seconds.trunc()).and_then(|value| value.to_i64());
    whole
        .and_then(|value| value.checked_mul(NANOS_PER_SECOND))
        .unwrap_or_else(|| {
            if seconds.is_sign_negative() {
                i64::MIN
            } else {
                i64::MAX
            }
        })
}

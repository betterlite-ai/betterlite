use std::time::SystemTime;

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::io_token_reservations::clock::seconds_until_minute_reset;
use crate::io_token_reservations::limits::IoLimits;

pub type RemainingUsage = IndexMap<String, i64>;

// the reset is read once before either side is looked at, so a minute edge cannot split the two headers
#[must_use]
// ast-grep-ignore: no-long-names
pub fn build_io_token_rate_limit_headers(
    limits: IoLimits,
    input_usage: Option<i64>,
    output_usage: Option<i64>,
    now: SystemTime,
) -> RemainingUsage {
    let reset = i64::try_from(seconds_until_minute_reset(now)).unwrap_or(i64::MAX);
    let mut headers = RemainingUsage::new();
    for (side, cap, usage) in [
        ("input", limits.itpm, input_usage),
        ("output", limits.otpm, output_usage),
    ] {
        let Some(configured) = cap else { continue };
        headers.insert(format!("x-ratelimit-limit-{side}-tokens"), configured);
        headers.insert(
            format!("x-ratelimit-remaining-{side}-tokens"),
            configured.saturating_sub(usage.unwrap_or(0)).max(0),
        );
        headers.insert(format!("x-ratelimit-reset-{side}-tokens"), reset);
    }
    headers
}

pub fn apply_remaining_usage_headers(
    additional_headers: &mut AdditionalHeaders,
    remaining: &RemainingUsage,
) {
    for (header, value) in remaining {
        additional_headers
            .entry(header.clone())
            .or_insert_with(|| Json::Number(Number::from(*value)));
    }
}

mod tests;

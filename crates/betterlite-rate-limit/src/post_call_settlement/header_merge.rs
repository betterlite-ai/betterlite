use serde_json::Number;

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;

use crate::limiter_records::RateLimitStatus;

mod tests;

// LiteLLM's _merge_ratelimit_statuses_into_additional_headers: a copy of the header map, extended with one
// x-ratelimit-{descriptor_key}-{remaining|limit}-{rate_limit_type} pair per status
#[must_use]
pub fn merge_ratelimit_statuses_into_additional_headers(
    additional_headers: &AdditionalHeaders,
    statuses: &[RateLimitStatus],
) -> AdditionalHeaders {
    let mut merged = additional_headers.clone();
    merge_statuses_into(&mut merged, statuses);
    merged
}

pub(crate) fn merge_statuses_into(merged: &mut AdditionalHeaders, statuses: &[RateLimitStatus]) {
    for status in statuses {
        let prefix = format!("x-ratelimit-{}", status.descriptor_key);
        let rate_limit_type = status.rate_limit_type.as_str();
        merged.insert(
            format!("{prefix}-remaining-{rate_limit_type}"),
            Json::Number(Number::from(status.limit_remaining)),
        );
        merged.insert(
            format!("{prefix}-limit-{rate_limit_type}"),
            Json::Number(Number::from(status.current_limit)),
        );
    }
}

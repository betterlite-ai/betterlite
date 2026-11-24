use betterlite_models::decisions::Json;
use num_traits::ToPrimitive as _;

use super::BatchLimitDescriptors;
use super::limiter::BatchLimiterSource;
use crate::api::request_destination::RequestData;

// Python's int() of a JSON cap: a number truncates toward zero, a numeric string parses whole
fn cap_value(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64().or_else(|| number.as_f64()?.to_i64()),
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Text(text) => text.trim().parse::<i64>().ok(),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

pub(super) fn estimate_output_tokens<Limiter, Credentials>(
    service: &BatchLimitDescriptors<Limiter, Credentials>,
    entry: &RequestData,
    min_configured_otpm_limit: Option<i64>,
) -> i64
where
    Limiter: BatchLimiterSource,
{
    if matches!(entry.get("url"), Some(Json::Text(url)) if url.contains("embeddings")) {
        return 0;
    }
    let empty = RequestData::new();
    let body: &RequestData = match entry.get("body") {
        Some(Json::Object(fields)) => fields,
        Some(Json::Text(_) | Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_))
        | None => &empty,
    };
    let cap_field = ["max_tokens", "max_completion_tokens", "max_output_tokens"]
        .into_iter()
        .find_map(|name| {
            body.get(name)
                .filter(|value| !matches!(**value, Json::Null))
        });
    let candidate_count = service.limiter.candidate_count(body);
    if let Some(cap) = cap_field.and_then(cap_value) {
        return cap.max(0).saturating_mul(candidate_count);
    }
    service
        .limiter
        .output_floor(min_configured_otpm_limit)
        .saturating_mul(candidate_count)
}

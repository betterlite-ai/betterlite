use std::ops::Div as _;

use num_traits::cast::cast;

use crate::limiter_records::{CacheCounterValue, RateLimitType, create_rate_limit_keys};
use crate::provider_attribution::ModelGroupLimits;

use super::cache::{SaturationCache, saturation_value_from_cache};
use super::descriptors::SATURATION_DESCRIPTOR_KEY;
use super::settings::PriorityPolicy;
use super::ttl::saturation_check_cache_ttl;

mod tests;

// _check_model_saturation: how full the model's request and token windows are, as the larger of the two ratios
#[must_use]
pub async fn check_model_saturation(
    model: &str,
    limits: &ModelGroupLimits,
    cache: &dyn SaturationCache,
    policy: &PriorityPolicy,
) -> f64 {
    let local_ttl = saturation_check_cache_ttl(policy);
    let mut saturation = 0.0_f64;
    if let Some(rpm) = limits.rpm.filter(|limit| *limit > 0) {
        let counter_key =
            create_rate_limit_keys(SATURATION_DESCRIPTOR_KEY, model, RateLimitType::Requests);
        if let Some(value) = saturation_value_from_cache(cache, &counter_key, local_ttl).await {
            let Some(current) = whole_counter(&value) else {
                tracing::error!(
                    "Error checking saturation for {model}: the requests counter is not whole"
                );
                return 0.0;
            };
            let capacity = cast::<i64, f64>(rpm).unwrap_or(f64::INFINITY);
            saturation = saturation.max(current.div(capacity));
        }
    }
    if let Some(tpm) = limits.tpm.filter(|limit| *limit > 0) {
        let counter_key =
            create_rate_limit_keys(SATURATION_DESCRIPTOR_KEY, model, RateLimitType::Tokens);
        if let Some(value) = saturation_value_from_cache(cache, &counter_key, local_ttl).await {
            let Some(current) = float_counter(&value) else {
                tracing::error!(
                    "Error checking saturation for {model}: the tokens counter is not a number"
                );
                return 0.0;
            };
            let capacity = cast::<i64, f64>(tpm).unwrap_or(f64::INFINITY);
            saturation = saturation.max(current.div(capacity));
        }
    }
    saturation
}

// int(counter_value): a float counter truncates toward zero, and text has to spell a whole number
fn whole_counter(value: &CacheCounterValue) -> Option<f64> {
    let current = match value {
        CacheCounterValue::Int(current) => *current,
        CacheCounterValue::Float(current) => cast::<f64, i64>(current.trunc())?,
        CacheCounterValue::Text(text) => text.trim().parse::<i64>().ok()?,
        CacheCounterValue::Bytes(bytes) => {
            str::from_utf8(bytes).ok()?.trim().parse::<i64>().ok()?
        }
    };
    cast::<i64, f64>(current)
}

// float(counter_value): text keeps its fraction, so a token counter that is not whole still counts
fn float_counter(value: &CacheCounterValue) -> Option<f64> {
    match value {
        CacheCounterValue::Int(current) => cast::<i64, f64>(*current),
        CacheCounterValue::Float(current) => Some(*current),
        CacheCounterValue::Text(text) => text.trim().parse::<f64>().ok(),
        CacheCounterValue::Bytes(bytes) => str::from_utf8(bytes).ok()?.trim().parse::<f64>().ok(),
    }
}

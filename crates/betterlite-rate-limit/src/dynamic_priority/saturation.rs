use std::ops::Div as _;

use num_traits::cast::cast;

use crate::limiter_records::{
    CacheCounterValue, RateLimitType, WindowRateLimitType, create_rate_limit_keys,
};
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
            let Some(current) = counter_number(&value, WindowRateLimitType::Requests) else {
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
            let Some(current) = counter_number(&value, WindowRateLimitType::Tokens) else {
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

// int() reads the requests counter and float() the tokens one, so only tokens keep a fraction
fn counter_number(value: &CacheCounterValue, counter: WindowRateLimitType) -> Option<f64> {
    let text = match value {
        CacheCounterValue::Int(current) => return cast::<i64, f64>(*current),
        CacheCounterValue::Float(current) if counter == WindowRateLimitType::Tokens => {
            return Some(*current);
        }
        CacheCounterValue::Float(current) => {
            return cast::<f64, i64>(current.trunc()).and_then(cast);
        }
        CacheCounterValue::Text(text) => text,
        CacheCounterValue::Bytes(bytes) => str::from_utf8(bytes).ok()?,
    };
    match counter {
        WindowRateLimitType::Requests => cast::<i64, f64>(text.trim().parse::<i64>().ok()?),
        WindowRateLimitType::Tokens => text.trim().parse::<f64>().ok(),
    }
}

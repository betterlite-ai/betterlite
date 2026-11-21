use std::ops::Mul as _;

use num_traits::cast::cast;

use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};
use crate::provider_attribution::ModelGroupLimits;

use super::allocation::priority_allocation;
use super::normalization::normalize_priority_weights;
use super::settings::PriorityPolicy;

mod tests;

// the two counter keys the v3 limiter counts under: one per priority pool, and one for the whole model
pub const PRIORITY_DESCRIPTOR_KEY: &str = "priority_model";
pub const SATURATION_DESCRIPTOR_KEY: &str = "model_saturation_check";

// _create_priority_based_descriptors: the pool this request may use, sized by its normalized share of the model's capacity
#[must_use]
pub fn priority_based_descriptors(
    policy: &PriorityPolicy,
    model: &str,
    priority: Option<&str>,
    model_info: Option<&ModelGroupLimits>,
    window_size: i64,
) -> Vec<RateLimitDescriptor> {
    if policy.reservation.is_none() {
        return Vec::new();
    }
    let Some(limits) = model_info else {
        return Vec::new();
    };
    let normalized = normalize_priority_weights(policy, Some(limits));
    let allocation = priority_allocation(policy, model, priority, &normalized, Some(limits));
    let tokens = limits
        .tpm
        .map(|tpm| reserved_capacity(tpm, allocation.weight));
    let requests = limits
        .rpm
        .map(|rpm| reserved_capacity(rpm, allocation.weight));
    if tokens.is_none() && requests.is_none() {
        return Vec::new();
    }
    vec![RateLimitDescriptor {
        key: PRIORITY_DESCRIPTOR_KEY.to_owned(),
        value: allocation.key,
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: requests,
            tokens_per_unit: tokens,
            max_parallel_requests: None,
            window_size: Some(window_size),
        }),
    }]
}

// int(model_capacity * priority_weight): the share truncates toward zero, as Python's int() does
fn reserved_capacity(capacity: i64, weight: f64) -> i64 {
    let capacity_float = cast::<i64, f64>(capacity);
    let scaled = capacity_float.unwrap_or(f64::INFINITY).mul(weight);
    match cast::<f64, i64>(scaled) {
        Some(reserved) => reserved,
        None => saturated_limit(scaled),
    }
}

// int() raises on a share past every integer; betterlite stops at the bound so the request still has a pool
const fn saturated_limit(scaled: f64) -> i64 {
    if scaled.is_sign_negative() {
        return i64::MIN;
    }
    i64::MAX
}

// _create_model_tracking_descriptor: the model-wide counter the saturation check reads, with the headroom a multiplier adds
#[must_use]
pub fn model_tracking_descriptor(
    model: &str,
    model_info: &ModelGroupLimits,
    high_limit_multiplier: i64,
    window_size: i64,
) -> RateLimitDescriptor {
    // an explicit 0 is a limit of zero; Python's truthiness would read it as no limit (docs/compatibility.md)
    RateLimitDescriptor {
        key: SATURATION_DESCRIPTOR_KEY.to_owned(),
        value: model.to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: model_info
                .rpm
                .map(|rpm| rpm.saturating_mul(high_limit_multiplier)),
            tokens_per_unit: model_info
                .tpm
                .map(|tpm| tpm.saturating_mul(high_limit_multiplier)),
            max_parallel_requests: None,
            window_size: Some(window_size),
        }),
    }
}

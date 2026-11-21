#![cfg(test)]

use super::{
    PRIORITY_DESCRIPTOR_KEY, SATURATION_DESCRIPTOR_KEY, model_tracking_descriptor,
    priority_based_descriptors, reserved_capacity,
};
use crate::dynamic_priority::tests::support::{percent, policy};
use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};
use crate::provider_attribution::ModelGroupLimits;

const WINDOW: i64 = 60;

const fn limits(rpm: Option<i64>, tpm: Option<i64>) -> ModelGroupLimits {
    ModelGroupLimits { rpm, tpm }
}

const fn rate_limit(requests: Option<i64>, tokens: Option<i64>) -> RateLimitDescriptorRateLimit {
    RateLimitDescriptorRateLimit {
        requests_per_unit: requests,
        tokens_per_unit: tokens,
        max_parallel_requests: None,
        window_size: Some(WINDOW),
    }
}

#[test]
fn no_reservation_builds_nothing() {
    let descriptors = priority_based_descriptors(
        &policy(None),
        "gpt-4",
        Some("prod"),
        Some(&limits(Some(100), Some(1000))),
        WINDOW,
    );
    assert!(descriptors.is_empty());
}

#[test]
fn no_model_info_builds_nothing() {
    let configured = policy(Some(&[("prod", percent(0.5))]));
    let descriptors = priority_based_descriptors(&configured, "gpt-4", Some("prod"), None, WINDOW);
    assert!(descriptors.is_empty());
}

#[test]
fn no_capacity_builds_nothing() {
    let configured = policy(Some(&[("prod", percent(0.5))]));
    let descriptors = priority_based_descriptors(
        &configured,
        "gpt-4",
        Some("prod"),
        Some(&limits(None, None)),
        WINDOW,
    );
    assert!(descriptors.is_empty());
}

#[test]
fn an_explicit_priority_scales_both_limits() {
    let configured = policy(Some(&[("prod", percent(0.5))]));
    let descriptors = priority_based_descriptors(
        &configured,
        "gpt-4",
        Some("prod"),
        Some(&limits(Some(100), Some(1000))),
        WINDOW,
    );
    assert_eq!(
        descriptors,
        vec![RateLimitDescriptor {
            key: PRIORITY_DESCRIPTOR_KEY.to_owned(),
            value: "gpt-4:prod".to_owned(),
            rate_limit: Some(rate_limit(Some(50), Some(500))),
        }]
    );
}

#[test]
fn one_configured_limit_still_builds_the_descriptor() {
    let configured = policy(Some(&[("prod", percent(0.5))]));
    let descriptors = priority_based_descriptors(
        &configured,
        "gpt-4",
        Some("prod"),
        Some(&limits(Some(100), None)),
        WINDOW,
    );
    assert_eq!(
        descriptors,
        vec![RateLimitDescriptor {
            key: PRIORITY_DESCRIPTOR_KEY.to_owned(),
            value: "gpt-4:prod".to_owned(),
            rate_limit: Some(rate_limit(Some(50), None)),
        }]
    );
}

#[test]
fn the_default_pool_scales_by_the_default_priority() {
    let configured = policy(Some(&[("prod", percent(0.5))]));
    let descriptors = priority_based_descriptors(
        &configured,
        "gpt-4",
        None,
        Some(&limits(Some(100), Some(1000))),
        WINDOW,
    );
    assert_eq!(
        descriptors,
        vec![RateLimitDescriptor {
            key: PRIORITY_DESCRIPTOR_KEY.to_owned(),
            value: "gpt-4:default_pool".to_owned(),
            rate_limit: Some(rate_limit(Some(25), Some(250))),
        }]
    );
}

#[test]
fn a_reserved_capacity_truncates_toward_zero() {
    assert_eq!(reserved_capacity(10, 0.25), 2);
    assert_eq!(reserved_capacity(100, 0.5), 50);
}

#[test]
fn a_reserved_capacity_past_the_integer_range_stops_at_the_bound() {
    assert_eq!(reserved_capacity(i64::MAX, 2.0), i64::MAX);
    assert_eq!(reserved_capacity(i64::MAX, -2.0), i64::MIN);
}

#[test]
fn the_tracking_descriptor_carries_both_limits() {
    let descriptor = model_tracking_descriptor("gpt-4", &limits(Some(100), Some(1000)), 1, WINDOW);
    assert_eq!(
        descriptor,
        RateLimitDescriptor {
            key: SATURATION_DESCRIPTOR_KEY.to_owned(),
            value: "gpt-4".to_owned(),
            rate_limit: Some(rate_limit(Some(100), Some(1000))),
        }
    );
}

#[test]
fn the_multiplier_raises_the_tracking_limits() {
    let descriptor = model_tracking_descriptor("gpt-4", &limits(Some(100), Some(1000)), 3, WINDOW);
    assert_eq!(
        descriptor.rate_limit,
        Some(rate_limit(Some(300), Some(3000)))
    );
}

#[test]
fn a_tracking_limit_past_the_integer_range_stops_at_the_bound() {
    let descriptor = model_tracking_descriptor("gpt-4", &limits(Some(i64::MAX), None), 2, WINDOW);
    assert_eq!(
        descriptor.rate_limit,
        Some(rate_limit(Some(i64::MAX), None))
    );
}

#[test]
fn an_explicit_zero_stays_a_tracking_limit() {
    let descriptor = model_tracking_descriptor("gpt-4", &limits(Some(0), None), 1, WINDOW);
    assert_eq!(descriptor.rate_limit, Some(rate_limit(Some(0), None)));
}

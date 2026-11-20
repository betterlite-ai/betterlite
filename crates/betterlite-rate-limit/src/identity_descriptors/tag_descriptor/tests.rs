#![cfg(test)]

use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit, TagRateLimit};

use super::tag_rate_limit_descriptor;

fn descriptor(
    rpm_limit: Option<i64>,
    tpm_limit: Option<i64>,
    window_size: i64,
) -> RateLimitDescriptor {
    tag_rate_limit_descriptor(
        "tag-a",
        TagRateLimit {
            rpm_limit,
            tpm_limit,
        },
        window_size,
    )
}

fn expected(
    requests_per_unit: Option<i64>,
    tokens_per_unit: Option<i64>,
    window_size: i64,
) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: "tag".to_owned(),
        value: "tag-a".to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit,
            tokens_per_unit,
            max_parallel_requests: None,
            window_size: Some(window_size),
        }),
    }
}

#[test]
fn builds_a_tag_descriptor_with_both_limits() {
    assert_eq!(
        descriptor(Some(10), Some(20), 60),
        expected(Some(10), Some(20), 60)
    );
}

#[test]
fn keeps_an_absent_tpm_as_null() {
    assert_eq!(descriptor(Some(10), None, 60), expected(Some(10), None, 60));
}

#[test]
fn keeps_an_absent_rpm_as_null() {
    assert_eq!(descriptor(None, Some(20), 60), expected(None, Some(20), 60));
}

#[test]
fn keeps_zero_limits_instead_of_dropping_them() {
    assert_eq!(
        descriptor(Some(0), Some(0), 60),
        expected(Some(0), Some(0), 60)
    );
}

#[test]
fn takes_the_window_size_from_the_handler() {
    assert_eq!(
        descriptor(Some(10), Some(20), 45),
        expected(Some(10), Some(20), 45)
    );
}

#[test]
fn leaves_the_parallel_request_ceiling_unset() {
    let built = descriptor(Some(10), Some(20), 60);
    assert_eq!(
        built.rate_limit.map(|limit| limit.max_parallel_requests),
        Some(None)
    );
}

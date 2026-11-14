#![cfg(test)]

use rstest::rstest;

use crate::limiter_records::{
    RateLimitDescriptor, RateLimitDescriptorRateLimit, WindowRateLimitType,
};

use super::{IncrementAmounts, build_descriptor_atomic_payload};

fn descriptor(
    requests_per_unit: Option<i64>,
    tokens_per_unit: Option<i64>,
    window_size: Option<i64>,
) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: "model".to_owned(),
        value: "gpt-4".to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit,
            tokens_per_unit,
            max_parallel_requests: None,
            window_size,
        }),
    }
}

fn amounts(requests: i64, tokens: i64) -> IncrementAmounts {
    IncrementAmounts { requests, tokens }
}

#[test]
fn payload_lists_a_window_and_counter_key_pair_for_each_limited_type() {
    let (keys, args, meta) = build_descriptor_atomic_payload(
        &descriptor(Some(10), Some(200), Some(90)),
        amounts(1, 5),
        60,
    );
    assert_eq!(
        keys,
        vec![
            "{model:gpt-4}:window",
            "{model:gpt-4}:requests",
            "{model:gpt-4}:window",
            "{model:gpt-4}:tokens",
        ]
    );
    assert_eq!(args, vec![10, 1, 90, 90, 200, 5, 90, 90]);
    assert_eq!(meta.len(), 2);
    assert_eq!(meta[0].rate_limit_type, WindowRateLimitType::Requests);
    assert_eq!(meta[0].counter_key, "{model:gpt-4}:requests");
    assert_eq!(meta[1].rate_limit_type, WindowRateLimitType::Tokens);
    assert_eq!(meta[1].counter_key, "{model:gpt-4}:tokens");
}

#[test]
fn payload_carries_the_per_counter_metadata() {
    let (_, _, meta) =
        build_descriptor_atomic_payload(&descriptor(Some(10), None, Some(90)), amounts(2, 0), 60);
    let counter = &meta[0];
    assert_eq!(counter.descriptor_key, "model");
    assert_eq!(counter.descriptor_value, "gpt-4");
    assert_eq!(counter.current_limit, 10);
    assert_eq!(counter.rate_limit_type, WindowRateLimitType::Requests);
    assert_eq!(counter.window_key, "{model:gpt-4}:window");
    assert_eq!(counter.counter_key, "{model:gpt-4}:requests");
    assert_eq!(counter.increment, 2);
    assert_eq!(counter.ttl, 90);
    assert_eq!(counter.window_size, 90);
}

#[rstest]
#[case::requests_only(Some(10), None, WindowRateLimitType::Requests)]
#[case::tokens_only(None, Some(200), WindowRateLimitType::Tokens)]
fn payload_skips_a_type_without_a_limit(
    #[case] requests_per_unit: Option<i64>,
    #[case] tokens_per_unit: Option<i64>,
    #[case] included: WindowRateLimitType,
) {
    let (keys, args, meta) = build_descriptor_atomic_payload(
        &descriptor(requests_per_unit, tokens_per_unit, Some(60)),
        amounts(1, 5),
        60,
    );
    assert_eq!(keys.len(), 2);
    assert_eq!(args.len(), 4);
    assert_eq!(meta.len(), 1);
    assert_eq!(meta[0].rate_limit_type, included);
}

#[test]
fn payload_skips_a_negative_increment() {
    let (keys, args, meta) = build_descriptor_atomic_payload(
        &descriptor(Some(10), Some(200), Some(60)),
        amounts(-1, 0),
        60,
    );
    assert_eq!(keys, vec!["{model:gpt-4}:window", "{model:gpt-4}:tokens"]);
    assert_eq!(args, vec![200, 0, 60, 60]);
    assert_eq!(meta.len(), 1);
    assert_eq!(meta[0].rate_limit_type, WindowRateLimitType::Tokens);
}

#[test]
fn payload_is_empty_when_no_type_has_a_limit() {
    let (keys, args, meta) =
        build_descriptor_atomic_payload(&descriptor(None, None, Some(60)), amounts(1, 1), 60);
    assert!(keys.is_empty());
    assert!(args.is_empty());
    assert!(meta.is_empty());
}

#[test]
fn payload_is_empty_without_a_rate_limit_object() {
    let (keys, args, meta) = build_descriptor_atomic_payload(
        &RateLimitDescriptor {
            key: "model".to_owned(),
            value: "gpt-4".to_owned(),
            rate_limit: None,
        },
        amounts(1, 1),
        60,
    );
    assert!(keys.is_empty());
    assert!(args.is_empty());
    assert!(meta.is_empty());
}

#[test]
fn payload_defaults_the_window_size_from_the_limiter() {
    let (_, args, meta) =
        build_descriptor_atomic_payload(&descriptor(Some(10), None, None), amounts(1, 0), 90);
    assert_eq!(args, vec![10, 1, 90, 90]);
    assert_eq!(meta[0].ttl, 90);
    assert_eq!(meta[0].window_size, 90);
}

#[test]
fn payload_uses_a_zero_window_size_as_it_is_written() {
    let (_, args, meta) =
        build_descriptor_atomic_payload(&descriptor(Some(10), None, Some(0)), amounts(1, 0), 90);
    assert_eq!(args, vec![10, 1, 0, 0]);
    assert_eq!(meta[0].window_size, 0);
}

#[test]
fn payload_includes_a_zero_or_negative_limit_as_litellm_does() {
    let (_, args, meta) = build_descriptor_atomic_payload(
        &descriptor(Some(0), Some(-1), Some(60)),
        amounts(0, 0),
        60,
    );
    assert_eq!(args, vec![0, 0, 60, 60, -1, 0, 60, 60]);
    assert_eq!(meta.len(), 2);
}

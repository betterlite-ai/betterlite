#![cfg(test)]

use crate::limiter_records::{ParallelRequestGauge, RateLimitDescriptor, WindowKeyMetadata};
use crate::windowed_admission::collect::CollectedWindowedKeys;
use crate::windowed_admission::tests::support::{
    HANDLER_WINDOW_SIZE, descriptor, harness, key_list, limits,
};

fn collect(descriptors: &[RateLimitDescriptor], skip_tpm_check: bool) -> CollectedWindowedKeys {
    let harness = harness();
    harness
        .limiter()
        .collect_windowed_keys_and_gauges(descriptors, skip_tpm_check)
}

fn expected_metadata(
    requests_limit: Option<i64>,
    tokens_limit: Option<i64>,
    window_size: i64,
) -> WindowKeyMetadata {
    WindowKeyMetadata {
        requests_limit,
        tokens_limit,
        window_size,
        descriptor_key: "api-key".to_owned(),
        descriptor_value: "k1".to_owned(),
    }
}

#[test]
fn requests_and_tokens_add_a_window_and_a_counter_key_each() {
    let descriptors = [descriptor(
        "api-key",
        "k1",
        Some(limits(Some(10), Some(20), None, None)),
    )];
    let collected = collect(&descriptors, false);
    assert_eq!(
        collected.keys_to_fetch,
        key_list(&[
            "{api-key:k1}:window",
            "{api-key:k1}:requests",
            "{api-key:k1}:window",
            "{api-key:k1}:tokens",
        ])
    );
    assert_eq!(
        collected.key_metadata.get("{api-key:k1}:window"),
        Some(&expected_metadata(Some(10), Some(20), HANDLER_WINDOW_SIZE))
    );
    assert!(collected.gauges.is_empty());
}

#[test]
fn skip_tpm_check_drops_the_tokens_limit() {
    let descriptors = [descriptor(
        "api-key",
        "k1",
        Some(limits(Some(10), Some(20), None, None)),
    )];
    let collected = collect(&descriptors, true);
    assert_eq!(
        collected.keys_to_fetch,
        key_list(&["{api-key:k1}:window", "{api-key:k1}:requests"])
    );
    assert_eq!(
        collected.key_metadata.get("{api-key:k1}:window"),
        Some(&expected_metadata(Some(10), None, HANDLER_WINDOW_SIZE))
    );
}

#[test]
fn a_gauge_only_descriptor_adds_a_gauge_and_no_keys() {
    let descriptors = [descriptor(
        "api-key",
        "k1",
        Some(limits(None, None, Some(5), None)),
    )];
    let collected = collect(&descriptors, false);
    assert!(collected.keys_to_fetch.is_empty());
    assert!(collected.key_metadata.is_empty());
    assert_eq!(
        collected.gauges,
        vec![ParallelRequestGauge {
            counter_key: "{api-key:k1}:max_parallel_requests".to_owned(),
            limit: 5,
            descriptor_key: "api-key".to_owned(),
        }]
    );
}

#[test]
fn a_descriptor_with_no_limits_adds_nothing() {
    let descriptors = [
        descriptor("api-key", "k1", None),
        descriptor("api-key", "k1", Some(limits(None, None, None, Some(30)))),
    ];
    let collected = collect(&descriptors, false);
    assert!(collected.keys_to_fetch.is_empty());
    assert!(collected.key_metadata.is_empty());
    assert!(collected.gauges.is_empty());
}

#[test]
fn the_descriptor_window_size_wins_and_zero_falls_back() {
    let descriptors = [
        descriptor(
            "api-key",
            "own",
            Some(limits(Some(1), None, None, Some(30))),
        ),
        descriptor(
            "api-key",
            "zero",
            Some(limits(Some(1), None, None, Some(0))),
        ),
        descriptor("api-key", "absent", Some(limits(Some(1), None, None, None))),
    ];
    let collected = collect(&descriptors, false);
    let window_size = |key: &str, value: &str| {
        collected
            .key_metadata
            .get(&format!("{{{key}:{value}}}:window"))
            .map(|metadata| metadata.window_size)
    };
    assert_eq!(window_size("api-key", "own"), Some(30));
    assert_eq!(window_size("api-key", "zero"), Some(HANDLER_WINDOW_SIZE));
    assert_eq!(window_size("api-key", "absent"), Some(HANDLER_WINDOW_SIZE));
}

#[test]
fn two_descriptors_with_one_key_and_value_share_the_later_metadata() {
    let descriptors = [
        descriptor("api-key", "k1", Some(limits(Some(10), None, None, None))),
        descriptor(
            "api-key",
            "k1",
            Some(limits(Some(99), Some(5), None, Some(30))),
        ),
    ];
    let collected = collect(&descriptors, false);
    assert_eq!(
        collected.keys_to_fetch,
        key_list(&[
            "{api-key:k1}:window",
            "{api-key:k1}:requests",
            "{api-key:k1}:window",
            "{api-key:k1}:requests",
            "{api-key:k1}:window",
            "{api-key:k1}:tokens",
        ])
    );
    assert_eq!(collected.key_metadata.len(), 1);
    assert_eq!(
        collected.key_metadata.get("{api-key:k1}:window"),
        Some(&expected_metadata(Some(99), Some(5), 30))
    );
}

#[test]
fn several_descriptors_keep_their_order_in_the_key_list() {
    let descriptors = [
        descriptor("team", "a", Some(limits(Some(1), None, None, None))),
        descriptor("api-key", "b", Some(limits(None, Some(2), None, None))),
    ];
    let collected = collect(&descriptors, false);
    assert_eq!(
        collected.keys_to_fetch,
        key_list(&[
            "{team:a}:window",
            "{team:a}:requests",
            "{api-key:b}:window",
            "{api-key:b}:tokens",
        ])
    );
    assert_eq!(collected.key_metadata.len(), 2);
    assert_eq!(
        collected.key_metadata.get("{api-key:b}:window"),
        Some(&WindowKeyMetadata {
            requests_limit: None,
            tokens_limit: Some(2),
            window_size: HANDLER_WINDOW_SIZE,
            descriptor_key: "api-key".to_owned(),
            descriptor_value: "b".to_owned(),
        })
    );
}

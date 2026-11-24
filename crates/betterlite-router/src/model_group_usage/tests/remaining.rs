use std::time::{Duration, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::model_group_info::group::ModelGroupInfo;
use crate::model_group_usage::error::GroupUsageError;
use crate::model_group_usage::remaining::group_remaining_usage;

use super::support::{
    FailingMetadata, FakeCache, FakeMetadata, MetadataFailure, answered, deployment, group_info,
    itpm_key, now, number, ordered, otpm_key, rpm_key, runtime, tpm_key,
};

#[test]
fn remaining_rpm() {
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        deployment("group", "b", "openai/gpt-4o-mini"),
    ];
    let cache = FakeCache::answering(answered(vec![
        Json::Null,
        Json::Null,
        number(10),
        number(30),
    ]));
    let metadata = FakeMetadata {
        answer: Some(group_info(None, Some(100), None, None)),
    };
    let headers = runtime()
        .block_on(group_remaining_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(
        ordered(&headers),
        [
            ("x-ratelimit-remaining-requests", 60_i64),
            ("x-ratelimit-limit-requests", 100_i64),
        ]
    );
    assert_eq!(
        cache.batches(),
        [vec![
            tpm_key("a", "openai/gpt-4o"),
            tpm_key("b", "openai/gpt-4o-mini"),
            rpm_key("a", "openai/gpt-4o"),
            rpm_key("b", "openai/gpt-4o-mini"),
        ]]
    );
}

#[test]
fn remaining_tpm() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(20), number(3)]));
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(50), None, None, None)),
    };
    let headers = runtime()
        .block_on(group_remaining_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(
        ordered(&headers),
        [
            ("x-ratelimit-remaining-tokens", 30_i64),
            ("x-ratelimit-limit-tokens", 50_i64),
        ]
    );
}

#[test]
fn input_output_usage() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(5), number(7)]));
    let metadata = FakeMetadata {
        answer: Some(group_info(None, None, Some(40), Some(30))),
    };
    let headers = runtime()
        .block_on(group_remaining_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(
        ordered(&headers),
        [
            ("x-ratelimit-limit-input-tokens", 40_i64),
            ("x-ratelimit-remaining-input-tokens", 35_i64),
            ("x-ratelimit-reset-input-tokens", 60_i64),
            ("x-ratelimit-limit-output-tokens", 30_i64),
            ("x-ratelimit-remaining-output-tokens", 23_i64),
            ("x-ratelimit-reset-output-tokens", 60_i64),
        ]
    );
    assert_eq!(
        cache.batches(),
        [vec![
            itpm_key("a", "openai/gpt-4o"),
            otpm_key("a", "openai/gpt-4o"),
        ]]
    );
}

#[test]
fn missing_counter() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![Json::Null, Json::Null]));
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(100), Some(10), None, None)),
    };
    let headers = runtime()
        .block_on(group_remaining_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(headers.get("x-ratelimit-remaining-tokens"), Some(&100_i64));
    assert_eq!(headers.get("x-ratelimit-remaining-requests"), Some(&10_i64));
}

#[test]
fn io_and_standard_usage_are_read_separately_and_merged() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(5), number(7)]));
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(100), Some(50), Some(40), Some(30))),
    };
    let headers = runtime()
        .block_on(group_remaining_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(
        ordered(&headers),
        [
            ("x-ratelimit-limit-input-tokens", 40_i64),
            ("x-ratelimit-remaining-input-tokens", 35_i64),
            ("x-ratelimit-reset-input-tokens", 60_i64),
            ("x-ratelimit-limit-output-tokens", 30_i64),
            ("x-ratelimit-remaining-output-tokens", 23_i64),
            ("x-ratelimit-reset-output-tokens", 60_i64),
            ("x-ratelimit-remaining-tokens", 95_i64),
            ("x-ratelimit-limit-tokens", 100_i64),
            ("x-ratelimit-remaining-requests", 43_i64),
            ("x-ratelimit-limit-requests", 50_i64),
        ]
    );
    assert_eq!(
        cache.batches(),
        [
            vec![
                itpm_key("a", "openai/gpt-4o"),
                otpm_key("a", "openai/gpt-4o"),
            ],
            vec![tpm_key("a", "openai/gpt-4o"), rpm_key("a", "openai/gpt-4o")],
        ]
    );
}

#[test]
fn ordinary_remaining_may_go_negative() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(25), Json::Null]));
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(10), None, None, None)),
    };
    let headers = runtime()
        .block_on(group_remaining_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(headers.get("x-ratelimit-remaining-tokens"), Some(&-15_i64));
}

#[test]
fn io_remaining_clamps_at_zero_and_the_reset_is_whole() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(25), Json::Null]));
    let metadata = FakeMetadata {
        answer: Some(group_info(None, None, Some(10), None)),
    };
    let start = UNIX_EPOCH
        .checked_add(Duration::from_secs(60))
        .expect("the minute edge");
    let headers = runtime()
        .block_on(group_remaining_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            start,
        ))
        .expect("the remaining usage");
    assert_eq!(
        ordered(&headers),
        [
            ("x-ratelimit-limit-input-tokens", 10_i64),
            ("x-ratelimit-remaining-input-tokens", 0_i64),
            ("x-ratelimit-reset-input-tokens", 60_i64),
        ]
    );
}

#[rstest]
#[case::unknown_group(None)]
#[case::no_limits(Some(group_info(None, None, None, None)))]
fn no_group_or_limits_reads_no_counters(#[case] answer: Option<ModelGroupInfo>) {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::failing();
    let metadata = FakeMetadata { answer };
    let headers = runtime()
        .block_on(group_remaining_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert!(headers.is_empty());
    assert!(cache.batches().is_empty());
}

#[test]
fn metadata_failure_precedes_the_counter_read() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(3), Json::Null]));
    let outcome = runtime().block_on(group_remaining_usage(
        "group",
        Some(&rows),
        &FailingMetadata,
        &cache,
        now(),
    ));
    assert!(matches!(
        outcome,
        Err(GroupUsageError::Metadata(MetadataFailure))
    ));
    assert!(cache.batches().is_empty());
}

#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::concurrency_gauges::PARALLEL_SLOT_TTL;
use crate::concurrency_gauges::cache::GaugeCacheValue;
use crate::limiter_records::{CacheCounterValue, RateLimitStatus, RateLimitType};
use crate::rate_limit_window_store::batch_rate_limiter_script::window_script_args;
use crate::windowed_admission::tests::support::{
    FixedClock, GENERATED_SLOT, GaugeWrite, HANDLER_WINDOW_SIZE, NOW, PlannedScript, ScriptCall,
    descriptor, gauge_key, harness, key_list, limits, mixed_plan, request, text,
};
use crate::windowed_admission::{SlotIdGenerator, UuidSlotIds, WindowedAdmissionError};

fn window_key() -> String {
    "{api-key:k1}:window".to_owned()
}

fn requests_key() -> String {
    "{api-key:k1}:requests".to_owned()
}

fn window_values() -> [Option<CacheCounterValue>; 2] {
    [
        Some(CacheCounterValue::Text("1700000000".to_owned())),
        Some(CacheCounterValue::Int(2)),
    ]
}

#[tokio::test]
async fn an_empty_request_is_ok_without_touching_the_cache() {
    let harness = harness();
    let limiter = harness.limiter();
    let response = limiter
        .should_rate_limit(&request(Vec::new()))
        .await
        .unwrap();
    assert_eq!(response.overall_code, "OK");
    assert!(response.statuses.is_empty());
    assert!(response.reservation_windows.is_none());
    assert!(harness.cache.batch_reads().is_empty());
    assert!(harness.gauges.batch_reads().is_empty());
}

#[tokio::test]
async fn a_clock_before_the_epoch_is_an_error() {
    let mut harness = harness();
    harness.clock = FixedClock::before_epoch();
    let limiter = harness.limiter();
    let error = limiter
        .should_rate_limit(&request(Vec::new()))
        .await
        .unwrap_err();
    assert!(matches!(error, WindowedAdmissionError::ClockBeforeEpoch(_)));
}

#[tokio::test]
async fn a_request_without_a_script_reads_the_in_memory_sliding_window() {
    let mut harness = harness();
    harness.script_enabled = false;
    let limiter = harness.limiter();
    let descriptors = vec![descriptor(
        "api-key",
        "k1",
        Some(limits(Some(10), None, None, None)),
    )];
    let response = limiter
        .should_rate_limit(&request(descriptors))
        .await
        .unwrap();
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 1);
    assert_eq!(response.statuses[0].current_limit, 10);
    assert!(harness.script.calls().is_empty());
}

#[tokio::test]
async fn the_local_mirror_over_limit_returns_before_redis_and_the_gauges() {
    let harness = harness();
    harness.cache.seed(
        &window_key(),
        CacheCounterValue::Text("1700000000".to_owned()),
    );
    harness
        .cache
        .seed(&requests_key(), CacheCounterValue::Int(11));
    let descriptors = vec![descriptor(
        "api-key",
        "k1",
        Some(limits(Some(10), None, Some(5), None)),
    )];
    let limiter = harness.limiter();
    let response = limiter
        .should_rate_limit(&request(descriptors))
        .await
        .unwrap();
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(response.statuses.len(), 1);
    assert_eq!(
        response.statuses[0].rate_limit_type,
        RateLimitType::Requests
    );
    assert_eq!(response.statuses[0].limit_remaining, -1);
    let keys = key_list(&[window_key().as_str(), requests_key().as_str()]);
    assert_eq!(harness.cache.batch_reads(), vec![(keys, true)]);
    assert!(harness.script.calls().is_empty());
    assert!(harness.cache.writes().is_empty());
    assert!(harness.gauges.batch_reads().is_empty());
    assert!(harness.gauges.writes().is_empty());
}

#[tokio::test]
async fn a_read_only_check_reads_the_redis_snapshot_and_does_not_increment() {
    let harness = harness();
    harness.cache.seed(
        &window_key(),
        CacheCounterValue::Text("1700000000".to_owned()),
    );
    harness
        .cache
        .seed(&requests_key(), CacheCounterValue::Int(3));
    let mut check = request(vec![descriptor(
        "api-key",
        "k1",
        Some(limits(Some(10), None, None, None)),
    )]);
    check.read_only = true;
    let limiter = harness.limiter();
    let response = limiter.should_rate_limit(&check).await.unwrap();
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses[0].limit_remaining, 7);
    let keys = key_list(&[window_key().as_str(), requests_key().as_str()]);
    assert_eq!(
        harness.cache.batch_reads(),
        vec![(keys.clone(), true), (keys, false)]
    );
    assert!(harness.cache.writes().is_empty());
    assert!(harness.script.calls().is_empty());
}

#[tokio::test]
async fn a_read_only_check_without_a_snapshot_synthesizes_a_zeroed_window() {
    let harness = harness();
    harness.cache.fail_batch_reads();
    let mut check = request(vec![descriptor(
        "api-key",
        "k1",
        Some(limits(Some(0), None, None, None)),
    )]);
    check.read_only = true;
    let limiter = harness.limiter();
    let response = limiter.should_rate_limit(&check).await.unwrap();
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(response.statuses.len(), 1);
    assert_eq!(response.statuses[0].current_limit, 0);
    assert_eq!(response.statuses[0].limit_remaining, 0);
    assert_eq!(harness.cache.batch_reads().len(), 2);
    assert!(harness.cache.writes().is_empty());
    assert!(harness.script.calls().is_empty());
}

#[tokio::test]
async fn the_script_path_writes_the_counter_back_before_the_window() {
    let mut harness = harness();
    harness.script = PlannedScript::new(vec![mixed_plan(&window_values())]);
    let limiter = harness.limiter();
    let descriptors = vec![descriptor(
        "api-key",
        "k1",
        Some(limits(Some(10), None, None, None)),
    )];
    let response = limiter
        .should_rate_limit(&request(descriptors))
        .await
        .unwrap();
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses[0].limit_remaining, 8);
    let writes = harness.cache.writes();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].key, requests_key());
    assert_eq!(writes[0].value, CacheCounterValue::Int(2));
    assert_eq!(writes[0].ttl, HANDLER_WINDOW_SIZE);
    assert!(writes[0].local_only);
    assert_eq!(writes[1].key, window_key());
    assert_eq!(
        writes[1].value,
        CacheCounterValue::Text("1700000000".to_owned())
    );
    assert_eq!(writes[1].ttl, HANDLER_WINDOW_SIZE);
    assert!(writes[1].local_only);
}

#[tokio::test]
async fn the_handler_window_size_drives_the_script_and_the_write_back() {
    let mut harness = harness();
    harness.script = PlannedScript::new(vec![mixed_plan(&window_values())]);
    let limiter = harness.limiter();
    let descriptors = vec![descriptor(
        "api-key",
        "k1",
        Some(limits(Some(10), None, None, Some(30))),
    )];
    let response = limiter
        .should_rate_limit(&request(descriptors))
        .await
        .unwrap();
    assert_eq!(response.overall_code, "OK");
    assert_eq!(
        harness.script.calls(),
        vec![ScriptCall {
            keys: key_list(&[window_key().as_str(), requests_key().as_str()]),
            args: window_script_args(NOW, HANDLER_WINDOW_SIZE),
        }]
    );
    assert!(
        harness
            .cache
            .writes()
            .iter()
            .all(|write| write.ttl == HANDLER_WINDOW_SIZE)
    );
}

#[test]
fn the_uuid_slot_generator_returns_32_lowercase_hex_characters() {
    let slot = UuidSlotIds.next_slot_id();
    assert_eq!(slot.len(), 32);
    assert!(
        slot.chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    );
}

#[tokio::test]
async fn a_write_back_of_mismatched_or_odd_pairs_is_an_error() {
    let harness = harness();
    let limiter = harness.limiter();
    let keys = key_list(&[window_key().as_str(), requests_key().as_str()]);
    let longer = vec![
        CacheCounterValue::Text("1700000000".to_owned()),
        CacheCounterValue::Int(1),
        CacheCounterValue::Int(2),
        CacheCounterValue::Int(3),
    ];
    let error = limiter.write_back(&keys, &longer).await.unwrap_err();
    assert!(matches!(
        error,
        WindowedAdmissionError::WindowPairCount { keys: 2, values: 4 }
    ));
    let odd_keys = key_list(&[
        "{api-key:k1}:window",
        "{api-key:k1}:requests",
        "{api-key:k1}:tokens",
    ]);
    let odd = vec![
        CacheCounterValue::Text("1700000000".to_owned()),
        CacheCounterValue::Int(1),
        CacheCounterValue::Int(2),
    ];
    let error = limiter.write_back(&odd_keys, &odd).await.unwrap_err();
    assert!(matches!(
        error,
        WindowedAdmissionError::WindowPairCount { keys: 3, values: 3 }
    ));
}

#[tokio::test]
async fn the_gauge_phase_appends_its_status_to_the_windowed_check() {
    let mut harness = harness();
    harness.script = PlannedScript::new(vec![mixed_plan(&window_values())]);
    let mut check = request(vec![descriptor(
        "api-key",
        "k1",
        Some(limits(Some(10), None, Some(5), None)),
    )]);
    check.parallel_slot_id = Some("given-slot".to_owned());
    let limiter = harness.limiter();
    let response = limiter.should_rate_limit(&check).await.unwrap();
    assert_eq!(response.overall_code, "OK");
    assert_eq!(
        response.statuses,
        vec![
            RateLimitStatus {
                code: "OK".to_owned(),
                current_limit: 10,
                limit_remaining: 8,
                rate_limit_type: RateLimitType::Requests,
                descriptor_key: "api-key".to_owned(),
                descriptor_value: Some("k1".to_owned()),
            },
            RateLimitStatus {
                code: "OK".to_owned(),
                current_limit: 5,
                limit_remaining: 4,
                rate_limit_type: RateLimitType::MaxParallelRequests,
                descriptor_key: "api-key".to_owned(),
                descriptor_value: None,
            },
        ]
    );
    assert_eq!(
        harness.gauges.writes(),
        vec![GaugeWrite {
            key: gauge_key(),
            value: GaugeCacheValue::Registry(BTreeMap::from([(
                "given-slot".to_owned(),
                1_700_000_000.0,
            )])),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(String::new()))]
#[tokio::test]
async fn a_missing_or_empty_slot_id_generates_one(#[case] slot: Option<String>) {
    let harness = harness();
    let mut check = request(vec![descriptor(
        "api-key",
        "k1",
        Some(limits(None, None, Some(5), None)),
    )]);
    check.parallel_slot_id = slot;
    let limiter = harness.limiter();
    let response = limiter.should_rate_limit(&check).await.unwrap();
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 1);
    let writes = harness.gauges.writes();
    assert_eq!(writes.len(), 1);
    let GaugeCacheValue::Registry(registry) = &writes[0].value else {
        panic!("the gauge write is a slot registry");
    };
    assert_eq!(
        registry.keys().cloned().collect::<Vec<String>>(),
        vec![GENERATED_SLOT.to_owned()]
    );
}

#[tokio::test]
async fn a_gauge_over_limit_drives_the_overall_code() {
    let mut harness = harness();
    harness.script = PlannedScript::new(vec![mixed_plan(&window_values())]);
    harness.gauges.seed(
        &gauge_key(),
        GaugeCacheValue::Counter(CacheCounterValue::Int(1)),
    );
    let limiter = harness.limiter();
    let descriptors = vec![descriptor(
        "api-key",
        "k1",
        Some(limits(Some(10), None, Some(1), None)),
    )];
    let response = limiter
        .should_rate_limit(&request(descriptors))
        .await
        .unwrap();
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(response.statuses.len(), 2);
    assert_eq!(response.statuses[0].code, "OK");
    assert_eq!(
        response.statuses[0].rate_limit_type,
        RateLimitType::Requests
    );
    assert_eq!(response.statuses[1].code, "OVER_LIMIT");
    assert_eq!(
        response.statuses[1].rate_limit_type,
        RateLimitType::MaxParallelRequests
    );
    assert_eq!(response.statuses[1].limit_remaining, 0);
    assert!(harness.gauges.writes().is_empty());
}

#[tokio::test]
async fn a_redis_counter_over_limit_returns_before_the_gauges() {
    let mut harness = harness();
    harness.script = PlannedScript::new(vec![mixed_plan(&[
        text("1700000000"),
        Some(CacheCounterValue::Int(11)),
    ])]);
    let descriptors = vec![descriptor(
        "api-key",
        "k1",
        Some(limits(Some(10), None, Some(5), None)),
    )];
    let limiter = harness.limiter();
    let response = limiter
        .should_rate_limit(&request(descriptors))
        .await
        .unwrap();
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(response.statuses.len(), 1);
    assert_eq!(response.statuses[0].limit_remaining, -1);
    assert_eq!(harness.cache.writes().len(), 2);
    assert!(harness.gauges.batch_reads().is_empty());
    assert!(harness.gauges.writes().is_empty());
}

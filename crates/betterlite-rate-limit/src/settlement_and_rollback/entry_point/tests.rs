#![cfg(test)]

use std::sync::Arc;

use crate::limiter_records::{RateLimitResponse, RateLimitType};
use crate::settlement_and_rollback::SettlementError;
use crate::settlement_and_rollback::tests::support::{
    FakeCache, FakeScript, FixedClock, amounts, descriptor, key_list, reply, settlement, store,
    unlimited,
};

const NOW: u64 = 1_700_000_000;

fn empty_response() -> RateLimitResponse {
    RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses: Vec::new(),
        reservation_windows: None,
    }
}

#[tokio::test]
async fn more_increment_amounts_than_descriptors_are_rejected() {
    let cache = FakeCache::empty();
    let windows = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows, &clock, None, None, false);
    let error = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), None)],
            &[amounts(1, 0), amounts(1, 0)],
        )
        .await
        .expect_err("the lengths differ");
    assert!(matches!(
        error,
        SettlementError::DescriptorCount {
            descriptors: 1,
            increments: 2
        }
    ));
}

#[tokio::test]
async fn more_descriptors_than_increment_amounts_are_rejected() {
    let cache = FakeCache::empty();
    let windows = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows, &clock, None, None, false);
    let error = settler
        .atomic_check_and_increment_by_n(
            &[
                descriptor("model", "gpt-4", Some(10), None),
                descriptor("model", "gpt-3.5", Some(10), None),
            ],
            &[amounts(1, 0)],
        )
        .await
        .expect_err("the lengths differ");
    assert!(matches!(
        error,
        SettlementError::DescriptorCount {
            descriptors: 2,
            increments: 1
        }
    ));
}

#[tokio::test]
async fn a_descriptor_without_a_rate_limit_settles_nothing() {
    let cache = FakeCache::empty();
    let windows = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows, &clock, None, None, false);
    let response = settler
        .atomic_check_and_increment_by_n(&[unlimited("model", "gpt-4")], &[amounts(1, 1)])
        .await
        .expect("an empty group list is not an error");
    assert_eq!(response, empty_response());
    assert!(cache.reads().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn a_descriptor_without_configured_limits_settles_nothing() {
    let cache = FakeCache::empty();
    let windows = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows, &clock, None, None, false);
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", None, None)],
            &[amounts(1, 1)],
        )
        .await
        .expect("an empty group list is not an error");
    assert_eq!(response, empty_response());
    assert!(cache.reads().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn a_descriptor_without_counters_is_filtered_before_dispatch() {
    let script = FakeScript::new(vec![reply(&[0, 1, 100])]);
    let cache = FakeCache::empty();
    let windows = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows, &clock, Some(Arc::clone(&script)), None, false);
    let response = settler
        .atomic_check_and_increment_by_n(
            &[
                unlimited("model", "empty"),
                descriptor("model", "gpt-4", Some(10), None),
            ],
            &[amounts(1, 1), amounts(1, 0)],
        )
        .await
        .expect("the remaining group settles");
    let calls = script.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls.first().map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:gpt-4}:window",
            "{model:gpt-4}:requests"
        ]))
    );
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 1);
    let status = response.statuses.first().expect("one status");
    assert_eq!(status.rate_limit_type, RateLimitType::Requests);
    assert_eq!(status.limit_remaining, 9);
}

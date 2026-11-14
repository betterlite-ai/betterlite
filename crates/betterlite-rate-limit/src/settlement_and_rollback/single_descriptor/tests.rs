#![cfg(test)]

use std::sync::Arc;

use crate::limiter_records::RateLimitType;
use crate::settlement_and_rollback::tests::support::{
    FakeCache, FakeScript, FixedClock, amounts, descriptor, key_list, reply, settlement, store,
};

const NOW: u64 = 1_700_000_000;

#[tokio::test]
async fn one_descriptor_settles_both_counters_from_the_script() {
    let script = FakeScript::new(vec![reply(&[0, 3, 100, 7, 100])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows_store, &clock, Some(script.clone()), None, false);
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), Some(100))],
            &[amounts(1, 2)],
        )
        .await
        .expect("the descriptor settles");
    let calls = script.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls.first().map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:gpt-4}:window",
            "{model:gpt-4}:requests",
            "{model:gpt-4}:window",
            "{model:gpt-4}:tokens",
        ]))
    );
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 2);
    assert_eq!(
        response.reservation_windows.iter().flatten().count(),
        2,
        "both counters reserve their window"
    );
}

#[tokio::test]
async fn an_over_limit_descriptor_names_the_counter_it_blocks() {
    let script = FakeScript::new(vec![reply(&[1, 1, 9, 10])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows_store, &clock, Some(script), None, false);
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), Some(100))],
            &[amounts(1, 2)],
        )
        .await
        .expect("the descriptor reports its limit");
    assert_eq!(response.overall_code, "OVER_LIMIT");
    let status = response.statuses.first().expect("one status");
    assert_eq!(status.rate_limit_type, RateLimitType::Requests);
    assert_eq!(status.current_limit, 10);
    assert_eq!(status.limit_remaining, 1);
}

#[tokio::test]
async fn an_over_limit_descriptor_refunds_nothing() {
    let script = FakeScript::new(vec![reply(&[1, 1, 9, 10])]);
    let refunds = FakeScript::new(vec![reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(script),
        Some(refunds.clone()),
        false,
    );
    settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), None)],
            &[amounts(1, 0)],
        )
        .await
        .expect("the descriptor reports its limit");
    assert!(refunds.calls().is_empty());
}

#[tokio::test]
async fn an_empty_reply_keeps_reservation_windows_absent() {
    let script = FakeScript::new(vec![reply(&[])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows_store, &clock, Some(script), None, false);
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), None)],
            &[amounts(1, 0)],
        )
        .await
        .expect("an empty reply decodes to an empty OK response");
    assert_eq!(response.overall_code, "OK");
    assert!(response.statuses.is_empty());
    assert!(response.reservation_windows.is_none());
}

#[tokio::test]
async fn a_failing_single_descriptor_falls_back_without_refunds() {
    let script = FakeScript::failing(vec![reply(&[0, 1, 100])], &[0]);
    let refunds = FakeScript::new(vec![reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(script),
        Some(refunds.clone()),
        false,
    );
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), Some(100))],
            &[amounts(1, 2)],
        )
        .await
        .expect("the single descriptor falls back to memory");
    assert!(refunds.calls().is_empty());
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 2);
    assert!(!cache.writes().is_empty());
}

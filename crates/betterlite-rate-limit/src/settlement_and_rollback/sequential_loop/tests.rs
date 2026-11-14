#![cfg(test)]

use std::sync::Arc;

use crate::limiter_records::{RateLimitResponse, ReservationBackend};
use crate::settlement_and_rollback::tests::support::{
    FakeCache, FakeScript, FixedClock, amounts, descriptor, key_list, reply, settlement, store,
};

const NOW: u64 = 1_700_000_000;

fn windows(response: &RateLimitResponse) -> Vec<(String, String, ReservationBackend)> {
    response
        .reservation_windows
        .iter()
        .flatten()
        .map(|window| {
            (
                window.counter_key.clone(),
                window.window_start.clone(),
                window.backend,
            )
        })
        .collect()
}

#[tokio::test]
async fn each_descriptor_runs_its_own_script_call_in_order() {
    let script = FakeScript::new(vec![reply(&[0, 1, 100]), reply(&[0, 2, 200, 300, 200])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows_store, &clock, Some(script.clone()), None, false);
    let response = settler
        .atomic_check_and_increment_by_n(
            &[
                descriptor("model", "gpt-4", Some(10), None),
                descriptor("user", "alice", Some(10), Some(100)),
            ],
            &[amounts(1, 0), amounts(1, 2)],
        )
        .await
        .expect("both descriptors settle");
    let calls = script.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls.first().map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:gpt-4}:window",
            "{model:gpt-4}:requests"
        ]))
    );
    assert_eq!(
        calls.get(1).map(|call| call.keys.clone()),
        Some(key_list(&[
            "{user:alice}:window",
            "{user:alice}:requests",
            "{user:alice}:window",
            "{user:alice}:tokens",
        ]))
    );
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 3);
    assert_eq!(
        windows(&response),
        vec![
            (
                "{model:gpt-4}:requests".to_owned(),
                "100".to_owned(),
                ReservationBackend::Redis
            ),
            (
                "{user:alice}:requests".to_owned(),
                "200".to_owned(),
                ReservationBackend::Redis
            ),
            (
                "{user:alice}:tokens".to_owned(),
                "200".to_owned(),
                ReservationBackend::Redis
            ),
        ]
    );
}

#[tokio::test]
async fn the_loop_stops_at_the_first_over_limit_group() {
    let script = FakeScript::new(vec![
        reply(&[0, 1, 100]),
        reply(&[1, 1, 9, 10]),
        reply(&[0, 1, 300]),
    ]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows_store, &clock, Some(script.clone()), None, false);
    let response = settler
        .atomic_check_and_increment_by_n(
            &[
                descriptor("model", "gpt-4", Some(10), None),
                descriptor("model", "gpt-3.5", Some(10), None),
                descriptor("model", "claude", Some(10), None),
            ],
            &[amounts(1, 0), amounts(1, 0), amounts(1, 0)],
        )
        .await
        .expect("the first two descriptors settle");
    assert_eq!(script.calls().len(), 2);
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(response.statuses.len(), 1);
    let status = response.statuses.first().expect("one status");
    assert_eq!(status.descriptor_value.as_deref(), Some("gpt-3.5"));
    assert_eq!(status.limit_remaining, 1);
    assert!(response.reservation_windows.is_none());
}

#[tokio::test]
async fn one_group_is_one_script_call() {
    let script = FakeScript::new(vec![reply(&[0, 3, 100])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows_store, &clock, Some(script.clone()), None, false);
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), None)],
            &[amounts(1, 0)],
        )
        .await
        .expect("the descriptor settles");
    assert_eq!(script.calls().len(), 1);
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 1);
}

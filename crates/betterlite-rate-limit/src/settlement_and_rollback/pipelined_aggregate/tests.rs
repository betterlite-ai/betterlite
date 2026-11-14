#![cfg(test)]

use std::sync::Arc;

use crate::limiter_records::RateLimitResponse;
use crate::settlement_and_rollback::tests::support::{
    FakeBatch, FakeCache, FakeScript, FixedClock, amounts, descriptor, pipelined_store, reply,
    settlement,
};

const NOW: u64 = 1_700_000_000;

fn windows(response: &RateLimitResponse) -> Vec<(String, String)> {
    response
        .reservation_windows
        .iter()
        .flatten()
        .map(|window| (window.counter_key.clone(), window.window_start.clone()))
        .collect()
}

#[tokio::test]
async fn ok_groups_aggregate_their_statuses_and_windows() {
    let batch = FakeBatch::new(
        vec![
            reply(&[0, 1, 100]),
            reply(&[0, 2, 200]),
            reply(&[0, 3, 300]),
        ],
        &[],
    );
    let cache = FakeCache::empty();
    let windows_store = pipelined_store(Arc::clone(&cache), Arc::clone(&batch));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(FakeScript::new(Vec::new())),
        None,
        false,
    );
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
        .expect("every group settles");
    assert_eq!(response.overall_code, "OK");
    assert_eq!(
        response
            .statuses
            .iter()
            .map(|status| status.descriptor_value.clone())
            .collect::<Vec<_>>(),
        vec![
            Some("gpt-4".to_owned()),
            Some("gpt-3.5".to_owned()),
            Some("claude".to_owned()),
        ]
    );
    assert_eq!(
        windows(&response),
        vec![
            ("{model:claude}:requests".to_owned(), "300".to_owned()),
            ("{model:gpt-3.5}:requests".to_owned(), "200".to_owned()),
            ("{model:gpt-4}:requests".to_owned(), "100".to_owned()),
        ]
    );
}

#[tokio::test]
async fn a_single_ok_group_is_returned_as_its_decoded_response() {
    let batch = FakeBatch::new(vec![reply(&[0, 5, 500])], &[]);
    let cache = FakeCache::empty();
    let windows_store = pipelined_store(Arc::clone(&cache), Arc::clone(&batch));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(FakeScript::new(Vec::new())),
        None,
        false,
    );
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), None)],
            &[amounts(1, 0)],
        )
        .await
        .expect("the single group settles");
    assert_eq!(response.statuses.len(), 1);
    assert_eq!(
        response
            .statuses
            .first()
            .map(|status| status.limit_remaining),
        Some(5)
    );
    assert_eq!(
        windows(&response),
        vec![("{model:gpt-4}:requests".to_owned(), "500".to_owned())]
    );
}

#[tokio::test]
async fn groups_with_both_counters_aggregate_all_windows() {
    let batch = FakeBatch::new(
        vec![reply(&[0, 1, 100, 2, 100]), reply(&[0, 3, 200, 4, 200])],
        &[],
    );
    let cache = FakeCache::empty();
    let windows_store = pipelined_store(Arc::clone(&cache), Arc::clone(&batch));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(FakeScript::new(Vec::new())),
        None,
        false,
    );
    let response = settler
        .atomic_check_and_increment_by_n(
            &[
                descriptor("model", "gpt-4", Some(10), Some(100)),
                descriptor("model", "gpt-3.5", Some(10), Some(100)),
            ],
            &[amounts(1, 2), amounts(1, 2)],
        )
        .await
        .expect("every group settles");
    assert_eq!(response.statuses.len(), 4);
    assert_eq!(
        windows(&response),
        vec![
            ("{model:gpt-3.5}:requests".to_owned(), "200".to_owned()),
            ("{model:gpt-3.5}:tokens".to_owned(), "200".to_owned()),
            ("{model:gpt-4}:requests".to_owned(), "100".to_owned()),
            ("{model:gpt-4}:tokens".to_owned(), "100".to_owned()),
        ]
    );
}

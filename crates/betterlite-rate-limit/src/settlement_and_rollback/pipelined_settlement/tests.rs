#![cfg(test)]

use std::sync::Arc;

use crate::settlement_and_rollback::tests::support::{
    FakeBatch, FakeCache, FakeScript, FixedClock, amounts, descriptor, key_list, pipelined_store,
    reply, settlement, text,
};

const NOW: u64 = 1_700_000_000;

#[tokio::test]
async fn every_group_is_enqueued_in_one_batch_round() {
    let batch = FakeBatch::new(
        vec![
            reply(&[0, 1, 100]),
            reply(&[0, 2, 200]),
            reply(&[0, 3, 300]),
        ],
        &[],
    );
    let direct = FakeScript::new(Vec::new());
    let cache = FakeCache::empty();
    let windows_store = pipelined_store(Arc::clone(&cache), Arc::clone(&batch));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows_store, &clock, Some(direct.clone()), None, false);
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
        .expect("the batch settles every group");
    let calls = batch.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls.first().map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:gpt-4}:window",
            "{model:gpt-4}:requests",
        ]))
    );
    assert_eq!(
        calls.first().map(|call| call.args.clone()),
        Some(vec![text("10"), text("1"), text("60"), text("60")])
    );
    assert!(direct.calls().is_empty());
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 3);
    assert_eq!(response.reservation_windows.iter().flatten().count(), 3);
}

#[tokio::test]
async fn the_batch_carries_both_counters_of_a_descriptor() {
    let batch = FakeBatch::new(vec![reply(&[0, 1, 100, 2, 200])], &[]);
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
            &[descriptor("model", "gpt-4", Some(10), Some(100))],
            &[amounts(1, 2)],
        )
        .await
        .expect("the batch settles both counters");
    let calls = batch.calls();
    assert_eq!(
        calls.first().map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:gpt-4}:window",
            "{model:gpt-4}:requests",
            "{model:gpt-4}:window",
            "{model:gpt-4}:tokens",
        ]))
    );
    assert_eq!(
        calls.first().map(|call| call.args.clone()),
        Some(vec![
            text("10"),
            text("1"),
            text("60"),
            text("60"),
            text("100"),
            text("2"),
            text("60"),
            text("60"),
        ])
    );
    assert_eq!(response.statuses.len(), 2);
    assert_eq!(response.reservation_windows.iter().flatten().count(), 2);
}

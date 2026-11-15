#![cfg(test)]

use crate::settlement_and_rollback::tests::support::{
    FakeBatch, RunMode, amounts, descriptor, key_list, reply, run_settlement, text,
};

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
    let run = run_settlement(
        RunMode::Batched(&batch),
        &[],
        false,
        &[
            descriptor("model", "gpt-4", Some(10), None),
            descriptor("model", "gpt-3.5", Some(10), None),
            descriptor("model", "claude", Some(10), None),
        ],
        &[amounts(1, 0), amounts(1, 0), amounts(1, 0)],
    )
    .await;
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
    assert!(run.direct.calls().is_empty());
    let response = run.outcome.expect("the batch settles every group");
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 3);
    assert_eq!(response.reservation_windows.iter().flatten().count(), 3);
}

#[tokio::test]
async fn the_batch_carries_both_counters_of_a_descriptor() {
    let batch = FakeBatch::new(vec![reply(&[0, 1, 100, 2, 200])], &[]);
    let run = run_settlement(
        RunMode::Batched(&batch),
        &[],
        false,
        &[descriptor("model", "gpt-4", Some(10), Some(100))],
        &[amounts(1, 2)],
    )
    .await;
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
    let response = run.outcome.expect("the batch settles both counters");
    assert_eq!(response.statuses.len(), 2);
    assert_eq!(response.reservation_windows.iter().flatten().count(), 2);
}

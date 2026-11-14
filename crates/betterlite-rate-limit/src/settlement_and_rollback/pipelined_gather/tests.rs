#![cfg(test)]

use std::sync::Arc;

use crate::limiter_records::ScriptReply;
use crate::settlement_and_rollback::tests::support::{
    FakeBatch, FakeCache, FakeScript, FixedClock, amounts, descriptor, pipelined_store, reply,
    settlement,
};

const NOW: u64 = 1_700_000_000;

#[tokio::test]
async fn a_batch_serves_every_group_in_one_round() {
    let batch = FakeBatch::new(vec![reply(&[0, 1, 100]), reply(&[0, 9, 200])], &[]);
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
            ],
            &[amounts(1, 0), amounts(1, 0)],
        )
        .await
        .expect("the batch settles both groups");
    assert_eq!(batch.calls().len(), 2);
    assert!(direct.calls().is_empty());
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 2);
    assert_eq!(
        response
            .statuses
            .iter()
            .map(|status| status.limit_remaining)
            .collect::<Vec<_>>(),
        vec![9, 1]
    );
    assert_eq!(response.reservation_windows.iter().flatten().count(), 2);
}

#[tokio::test]
async fn a_group_failure_does_not_stop_the_other_groups_reading() {
    let batch = FakeBatch::new(vec![reply(&[0, 1, 100]), reply(&[0, 2, 200])], &[1]);
    let refunds = FakeScript::new(vec![reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = pipelined_store(Arc::clone(&cache), Arc::clone(&batch));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(FakeScript::new(Vec::new())),
        Some(refunds.clone()),
        false,
    );
    let response = settler
        .atomic_check_and_increment_by_n(
            &[
                descriptor("model", "gpt-4", Some(10), None),
                descriptor("model", "gpt-3.5", Some(10), None),
            ],
            &[amounts(1, 0), amounts(1, 0)],
        )
        .await
        .expect("the failed group falls back to memory");
    assert_eq!(batch.calls().len(), 2);
    assert_eq!(refunds.calls().len(), 1);
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 2);
    assert!(!cache.writes().is_empty());
}

#[tokio::test]
async fn an_unreadable_reply_is_that_groups_failure() {
    let batch = FakeBatch::new(vec![reply(&[0, 1, 100]), ScriptReply::Int(7)], &[]);
    let refunds = FakeScript::new(vec![reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = pipelined_store(Arc::clone(&cache), Arc::clone(&batch));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(FakeScript::new(Vec::new())),
        Some(refunds.clone()),
        false,
    );
    let response = settler
        .atomic_check_and_increment_by_n(
            &[
                descriptor("model", "gpt-4", Some(10), None),
                descriptor("model", "gpt-3.5", Some(10), None),
            ],
            &[amounts(1, 0), amounts(1, 0)],
        )
        .await
        .expect("the unreadable group falls back to memory");
    assert_eq!(refunds.calls().len(), 1);
    assert_eq!(response.statuses.len(), 2);
}

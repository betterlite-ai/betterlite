#![cfg(test)]

use std::sync::Arc;

use crate::settlement_and_rollback::tests::support::{
    FakeBatch, FakeCache, FakeScript, FixedClock, amounts, descriptor, key_list, pipelined_store,
    reply, settlement, text,
};

const NOW: u64 = 1_700_000_000;

#[tokio::test]
async fn an_over_limit_group_refunds_the_groups_that_ran_after_it() {
    let batch = FakeBatch::new(
        vec![
            reply(&[1, 1, 9, 10]),
            reply(&[0, 4, 1000, 5, 1000]),
            reply(&[0, 6, 2000, 7, 2000]),
        ],
        &[],
    );
    let refunds = FakeScript::new(vec![reply(&[1]), reply(&[1]), reply(&[1]), reply(&[1])]);
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
                descriptor("model", "gpt-4", Some(10), Some(100)),
                descriptor("model", "gpt-3.5", Some(10), Some(100)),
                descriptor("model", "claude", Some(10), Some(100)),
            ],
            &[amounts(1, 2), amounts(1, 2), amounts(1, 2)],
        )
        .await
        .expect("the over-limit group ends the settlement");
    let refund_calls = refunds.calls();
    assert_eq!(refund_calls.len(), 4);
    assert_eq!(
        refund_calls.first().map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:gpt-3.5}:window",
            "{model:gpt-3.5}:requests"
        ]))
    );
    assert_eq!(
        refund_calls.first().map(|call| call.args.clone()),
        Some(vec![text("1000"), text("-1"), text("0")])
    );
    assert_eq!(
        refund_calls.get(1).map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:gpt-3.5}:window",
            "{model:gpt-3.5}:tokens"
        ]))
    );
    assert_eq!(
        refund_calls.get(3).map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:claude}:window",
            "{model:claude}:tokens"
        ]))
    );
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(response.statuses.len(), 1);
    assert!(response.reservation_windows.is_none());
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn an_over_limit_group_wins_over_a_coexisting_failure() {
    let batch = FakeBatch::new(vec![reply(&[0, 1, 100]), reply(&[1, 2, 9, 10])], &[0]);
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
                descriptor("model", "gpt-3.5", Some(10), Some(100)),
            ],
            &[amounts(1, 0), amounts(1, 0)],
        )
        .await
        .expect("the over-limit verdict stands over the failure");
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(response.statuses.len(), 1);
    let status = response.statuses.first().expect("one status");
    assert_eq!(status.descriptor_value.as_deref(), Some("gpt-3.5"));
    assert!(refunds.calls().is_empty());
    assert!(cache.reads().is_empty());
}

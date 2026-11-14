#![cfg(test)]

use std::sync::Arc;

use crate::settlement_and_rollback::tests::support::{
    FakeCache, FakeScript, FixedClock, amounts, descriptor, key_list, reply, settlement, store,
    text,
};

const NOW: u64 = 1_700_000_000;

#[tokio::test]
async fn an_over_limit_group_refunds_the_groups_before_it() {
    let script = FakeScript::new(vec![reply(&[0, 4, 1000, 5, 1000]), reply(&[1, 1, 9, 10])]);
    let refunds = FakeScript::new(vec![reply(&[1]), reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(script.clone()),
        Some(refunds.clone()),
        false,
    );
    let response = settler
        .atomic_check_and_increment_by_n(
            &[
                descriptor("model", "gpt-4", Some(10), Some(100)),
                descriptor("model", "gpt-3.5", Some(10), None),
            ],
            &[amounts(1, 2), amounts(1, 0)],
        )
        .await
        .expect("the over-limit group ends the settlement");
    assert_eq!(script.calls().len(), 2);
    let refund_calls = refunds.calls();
    assert_eq!(refund_calls.len(), 2);
    assert_eq!(
        refund_calls.first().map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:gpt-4}:window",
            "{model:gpt-4}:requests"
        ]))
    );
    assert_eq!(
        refund_calls.first().map(|call| call.args.clone()),
        Some(vec![text("1000"), text("-1"), text("0")])
    );
    assert_eq!(
        refund_calls.get(1).map(|call| call.keys.clone()),
        Some(key_list(&["{model:gpt-4}:window", "{model:gpt-4}:tokens"]))
    );
    assert_eq!(
        refund_calls.get(1).map(|call| call.args.clone()),
        Some(vec![text("1000"), text("-2"), text("0")])
    );
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(response.statuses.len(), 1);
    let status = response.statuses.first().expect("one status");
    assert_eq!(status.descriptor_value.as_deref(), Some("gpt-3.5"));
    assert_eq!(status.limit_remaining, 1);
    assert!(response.reservation_windows.is_none());
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn an_over_limit_first_group_stops_the_loop_without_refunds() {
    let script = FakeScript::new(vec![reply(&[1, 1, 9, 10]), reply(&[0, 1, 100])]);
    let refunds = FakeScript::new(vec![reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(script.clone()),
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
        .expect("the first group reports its limit");
    assert_eq!(script.calls().len(), 1);
    assert!(refunds.calls().is_empty());
    assert_eq!(response.overall_code, "OVER_LIMIT");
}

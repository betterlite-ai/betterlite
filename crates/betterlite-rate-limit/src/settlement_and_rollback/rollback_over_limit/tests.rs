#![cfg(test)]

use std::sync::Arc;

use crate::settlement_and_rollback::tests::support::{
    FakeCache, FakeScript, FixedClock, RunMode, amounts, descriptor, expected_refund_calls,
    recorded_calls, reply, run_settlement, settlement, store,
};

const NOW: u64 = 1_700_000_000;

const REFUNDED_GROUPS: &[(&str, &str, &str, &str)] = &[
    (
        "{model:gpt-4}:window",
        "{model:gpt-4}:requests",
        "1000",
        "-1",
    ),
    ("{model:gpt-4}:window", "{model:gpt-4}:tokens", "1000", "-2"),
];

#[tokio::test]
async fn an_over_limit_group_refunds_the_groups_before_it() {
    let script = FakeScript::new(vec![reply(&[0, 4, 1000, 5, 1000]), reply(&[1, 1, 9, 10])]);
    let run = run_settlement(
        RunMode::Scripted(Arc::clone(&script)),
        &[reply(&[1]), reply(&[1])],
        false,
        &[
            descriptor("model", "gpt-4", Some(10), Some(100)),
            descriptor("model", "gpt-3.5", Some(10), None),
        ],
        &[amounts(1, 2), amounts(1, 0)],
    )
    .await;
    let response = run.outcome.expect("the over-limit group settles");
    assert_eq!(script.calls().len(), 2);
    assert_eq!(
        recorded_calls(&run.refunds),
        expected_refund_calls(REFUNDED_GROUPS)
    );
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(response.statuses.len(), 1);
    let status = response.statuses.first().expect("one status");
    assert_eq!(status.descriptor_value.as_deref(), Some("gpt-3.5"));
    assert_eq!(status.limit_remaining, 1);
    assert!(response.reservation_windows.is_none());
    assert!(run.cache.reads().is_empty());
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
        Some(Arc::clone(&script)),
        Some(Arc::clone(&refunds)),
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

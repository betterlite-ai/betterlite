#![cfg(test)]

use std::sync::Arc;

use crate::settlement_and_rollback::SettlementError;
use crate::settlement_and_rollback::tests::support::{
    FakeBatch, FakeCache, FakeScript, FixedClock, LogCapture, amounts, descriptor, key_list,
    pipelined_store, reply, settlement, text,
};

const NOW: u64 = 1_700_000_000;

#[tokio::test]
async fn a_failed_group_refunds_the_ok_groups_and_falls_back() {
    let capture = LogCapture::start();
    let batch = FakeBatch::new(vec![reply(&[0, 1, 100]), reply(&[0, 2, 200])], &[0]);
    let refunds = FakeScript::new(vec![reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = pipelined_store(Arc::clone(&cache), Arc::clone(&batch));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(FakeScript::new(Vec::new())),
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
        .expect("the failed group falls back to memory");
    let refund_calls = refunds.calls();
    assert_eq!(refund_calls.len(), 1);
    assert_eq!(
        refund_calls.first().map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:gpt-3.5}:window",
            "{model:gpt-3.5}:requests"
        ]))
    );
    assert_eq!(
        refund_calls.first().map(|call| call.args.clone()),
        Some(vec![text("200"), text("-1"), text("0")])
    );
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 2);
    assert!(!cache.writes().is_empty());
    let logged = capture.text();
    assert!(logged.contains("ERROR"));
    assert!(logged.contains("script=\"check_and_increment_by_n_script\""));
    assert!(logged.contains("applied=1"));
}

#[tokio::test]
async fn fail_closed_pipelined_failure_refunds_then_rejects() {
    let batch = FakeBatch::new(vec![reply(&[0, 1, 100]), reply(&[0, 2, 200])], &[1]);
    let refunds = FakeScript::new(vec![reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = pipelined_store(Arc::clone(&cache), Arc::clone(&batch));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(FakeScript::new(Vec::new())),
        Some(Arc::clone(&refunds)),
        true,
    );
    let error = settler
        .atomic_check_and_increment_by_n(
            &[
                descriptor("model", "gpt-4", Some(10), None),
                descriptor("model", "gpt-3.5", Some(10), None),
            ],
            &[amounts(1, 0), amounts(1, 0)],
        )
        .await
        .expect_err("fail-closed rejects an unverifiable settlement");
    let SettlementError::Unverifiable(inner) = error else {
        panic!("the rejection is the fail-closed error");
    };
    assert_eq!(inner.status_code, 503);
    assert_eq!(refunds.calls().len(), 1);
    assert!(cache.reads().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn a_failure_in_the_only_group_falls_back_without_refunds() {
    let capture = LogCapture::start();
    let batch = FakeBatch::new(vec![reply(&[0, 1, 100])], &[0]);
    let refunds = FakeScript::new(vec![reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = pipelined_store(Arc::clone(&cache), Arc::clone(&batch));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(FakeScript::new(Vec::new())),
        Some(Arc::clone(&refunds)),
        false,
    );
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), None)],
            &[amounts(1, 0)],
        )
        .await
        .expect("the only group falls back to memory");
    assert!(refunds.calls().is_empty());
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 1);
    assert!(capture.text().contains("applied=0"));
}

#![cfg(test)]

use std::sync::Arc;

use crate::settlement_and_rollback::SettlementError;
use crate::settlement_and_rollback::tests::support::{
    FakeCache, FakeScript, FixedClock, LogCapture, amounts, descriptor, key_list, reply,
    settlement, store, text,
};

const NOW: u64 = 1_700_000_000;

#[tokio::test]
async fn a_failed_script_refunds_applied_groups_and_falls_back_to_memory() {
    let capture = LogCapture::start();
    let script = FakeScript::failing(vec![reply(&[0, 1, 100]), reply(&[0, 1, 200])], &[1]);
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
        .expect("the settlement falls back to memory");
    assert_eq!(script.calls().len(), 2);
    let refund_calls = refunds.calls();
    assert_eq!(refund_calls.len(), 1);
    assert_eq!(
        refund_calls.first().map(|call| call.keys.clone()),
        Some(key_list(&[
            "{model:gpt-4}:window",
            "{model:gpt-4}:requests"
        ]))
    );
    assert_eq!(
        refund_calls.first().map(|call| call.args.clone()),
        Some(vec![text("100"), text("-1"), text("0")])
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
async fn fail_closed_rejects_after_the_refunds_and_before_the_fallback() {
    let script = FakeScript::failing(vec![reply(&[0, 1, 100]), reply(&[0, 1, 200])], &[1]);
    let refunds = FakeScript::new(vec![reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(script),
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
async fn a_failure_before_any_increment_refunds_nothing() {
    let capture = LogCapture::start();
    let script = FakeScript::failing(vec![reply(&[0, 1, 100])], &[0]);
    let refunds = FakeScript::new(vec![reply(&[1])]);
    let cache = FakeCache::empty();
    let windows_store = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(
        &windows_store,
        &clock,
        Some(script),
        Some(Arc::clone(&refunds)),
        false,
    );
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), None)],
            &[amounts(1, 0)],
        )
        .await
        .expect("the settlement falls back to memory");
    assert!(refunds.calls().is_empty());
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 1);
    assert!(capture.text().contains("applied=0"));
}

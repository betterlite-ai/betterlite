#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::post_call_settlement::failure_hook::FailureHook;
use crate::post_call_settlement::tests::support::{
    BatchCall, LogCapture, SettlementFixture, WINDOW, install, operation_tuples, reservation,
    scopes, stash_state, unguarded_operation,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn refunds_batch_reservations_before_the_release_guard() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reservation_released = true;
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
        state.batch_enqueued_reservation = Some(reservation(500, &[("api_key", "k1")]));
        state.batch_tpd_refund_ops = vec![unguarded_operation("{api_key:k1}:tokens", -50.0)];
    });
    let event = FailureHook {
        stash: &slot,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().post_call_failure_hook(&event).await;
    assert_eq!(
        fixture.store.calls(),
        vec![BatchCall {
            kind: "refund",
            batch_id: None,
            tokens: Some(500),
        }]
    );
    assert_eq!(
        operation_tuples(&fixture.usage.increments()),
        vec![(
            "{api_key:k1}:tokens".to_owned(),
            (-50.0_f64).to_bits(),
            Some(WINDOW),
        )]
    );
    assert!(fixture.usage.pipelines().is_empty());
    let state = stash_state(&slot);
    assert!(state.batch_enqueued_reservation.is_none());
    assert!(state.batch_tpd_refund_ops.is_empty());
    assert!(state.reservation_released);
}

#[tokio::test]
async fn stops_at_a_failed_batch_refund() {
    let capture = LogCapture::start();
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
        state.batch_enqueued_reservation = Some(reservation(500, &[("api_key", "k1")]));
        state.batch_tpd_refund_ops = vec![unguarded_operation("{api_key:k1}:tokens", -50.0)];
    });
    fixture.store.fail_only("refund");
    let event = FailureHook {
        stash: &slot,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().post_call_failure_hook(&event).await;
    assert!(
        capture
            .text()
            .contains("Error releasing TPM reservation on post-call failure")
    );
    assert_eq!(fixture.store.calls().len(), 1);
    assert!(fixture.usage.pipelines().is_empty());
    assert!(fixture.usage.increments().is_empty());
    let state = stash_state(&slot);
    assert!(state.batch_enqueued_reservation.is_some());
    assert_eq!(state.batch_tpd_refund_ops.len(), 1);
    assert!(!state.reservation_released);
}

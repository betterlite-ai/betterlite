#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::post_call_settlement::failure_event::FailureEvent;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, WINDOW, install, scopes, stash_state,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn skips_the_refund_when_the_reservation_was_already_released() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reservation_released = true;
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
    });
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    assert!(fixture.usage.pipelines().is_empty());
    assert!(stash_state(&slot).reservation_released);
}

#[tokio::test]
async fn refunds_once_when_the_callback_runs_twice() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
    });
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    fixture.settlement().log_failure_event(&event).await;
    assert_eq!(fixture.usage.pipelines().len(), 1);
    assert!(stash_state(&slot).reservation_released);
}

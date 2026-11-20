#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::post_call_settlement::failure_event::FailureEvent;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, WINDOW, install, scopes, stash_state, token_usage,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn flags_the_reservation_even_when_the_delta_is_zero() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
    });
    let usage = token_usage(600, 400, 1000);
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: Some(&usage),
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    assert!(fixture.usage.pipelines().is_empty());
    assert!(stash_state(&slot).reservation_released);
}

#[tokio::test]
async fn flags_an_output_only_reservation() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.otpm_reserved_tokens = 300;
        state.otpm_reserved_scopes = scopes(&[("project", "p2")]);
    });
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    assert!(stash_state(&slot).reservation_released);
}

#[tokio::test]
async fn leaves_the_flag_alone_without_a_reservation() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |_state| {});
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    assert!(!stash_state(&slot).reservation_released);
}

#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::post_call_settlement::failure_hook::FailureHook;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, WINDOW, install, scopes, stash_state,
};
use crate::request_stash::RequestStashSlot;
use crate::usage_resolution::Usage;

#[tokio::test]
async fn flags_the_reservation_after_the_refund() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
    });
    let event = FailureHook {
        stash: &slot,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().post_call_failure_hook(&event).await;
    assert!(stash_state(&slot).reservation_released);
}

#[tokio::test]
async fn flags_the_reservation_even_when_the_delta_is_zero() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
    });
    let usage = Usage {
        prompt_tokens: 600,
        completion_tokens: 400,
        total_tokens: 1000,
        ..Usage::default()
    };
    let event = FailureHook {
        stash: &slot,
        combined_usage: Some(&usage),
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().post_call_failure_hook(&event).await;
    assert!(fixture.usage.pipelines().is_empty());
    assert!(stash_state(&slot).reservation_released);
}

#[tokio::test]
async fn leaves_the_flag_alone_without_a_reservation() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |_state| {});
    let event = FailureHook {
        stash: &slot,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().post_call_failure_hook(&event).await;
    assert!(!stash_state(&slot).reservation_released);
}

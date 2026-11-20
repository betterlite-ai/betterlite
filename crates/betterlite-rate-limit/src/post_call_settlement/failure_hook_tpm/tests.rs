#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::post_call_settlement::failure_hook::FailureHook;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, WINDOW, install, pipeline_tuples, scopes, stash_state,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn refunds_the_combined_reservation_on_rejection() {
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
    assert_eq!(
        pipeline_tuples(&fixture.usage.pipelines()),
        vec![vec![(
            "{user:alice}:tokens".to_owned(),
            (-1000.0_f64).to_bits(),
            Some(WINDOW),
        )]]
    );
    assert!(stash_state(&slot).reservation_released);
}

#[tokio::test]
async fn skips_the_refund_when_the_flag_is_set() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reservation_released = true;
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
    assert!(fixture.usage.pipelines().is_empty());
    assert!(stash_state(&slot).reservation_released);
}

#[tokio::test]
async fn refunds_nothing_without_a_reservation() {
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
    assert!(fixture.usage.pipelines().is_empty());
    assert!(!stash_state(&slot).reservation_released);
}

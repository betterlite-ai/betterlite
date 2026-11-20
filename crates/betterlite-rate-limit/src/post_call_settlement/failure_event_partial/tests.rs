#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::post_call_settlement::failure_event::FailureEvent;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, WINDOW, install, pipeline_tuples, scopes, stash_state, token_usage,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn settles_a_partial_stream_at_the_recovered_usage() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
    });
    let usage = token_usage(120, 30, 150);
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: Some(&usage),
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    assert_eq!(
        pipeline_tuples(&fixture.usage.pipelines()),
        vec![vec![(
            "{user:alice}:tokens".to_owned(),
            (-850.0_f64).to_bits(),
            Some(WINDOW),
        )]]
    );
    assert!(stash_state(&slot).reservation_released);
}

#[tokio::test]
async fn refunds_fully_when_no_completion_was_recovered() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
    });
    let usage = token_usage(120, 0, 120);
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: Some(&usage),
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    assert_eq!(
        pipeline_tuples(&fixture.usage.pipelines()),
        vec![vec![(
            "{user:alice}:tokens".to_owned(),
            (-1000.0_f64).to_bits(),
            Some(WINDOW),
        )]]
    );
}

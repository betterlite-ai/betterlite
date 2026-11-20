#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::post_call_settlement::failure_event::FailureEvent;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, WINDOW, install, pipeline_tuples, scopes, stash_state,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn refunds_each_scope_the_reservation_charged() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("team", "t1"), ("user", "alice")]);
    });
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    assert_eq!(
        pipeline_tuples(&fixture.usage.pipelines()),
        vec![vec![
            (
                "{team:t1}:tokens".to_owned(),
                (-1000.0_f64).to_bits(),
                Some(WINDOW),
            ),
            (
                "{user:alice}:tokens".to_owned(),
                (-1000.0_f64).to_bits(),
                Some(WINDOW),
            ),
        ]]
    );
}

#[tokio::test]
async fn refunds_nothing_when_the_reservation_charged_no_scope() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reserved_tokens = 1000;
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

#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::limiter_records::ReservationBackend;
use crate::post_call_settlement::failure_event::FailureEvent;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, WINDOW, install, operation_tuples, pipeline_tuples, scopes, stash_state,
    token_usage, window_identity,
};
use crate::request_stash::RequestStashSlot;

fn install_combined_and_both_projects(slot: &RequestStashSlot) {
    install(slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
        state.itpm_reserved_tokens = 200;
        state.itpm_reserved_scopes = scopes(&[("project", "p1")]);
        state.otpm_reserved_tokens = 300;
        state.otpm_reserved_scopes = scopes(&[("project", "p2")]);
    });
}

#[tokio::test]
async fn applies_combined_and_project_refunds_through_the_pipeline() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install_combined_and_both_projects(&slot);
    let usage = token_usage(100, 40, 140);
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
        vec![
            vec![(
                "{user:alice}:tokens".to_owned(),
                (-860.0_f64).to_bits(),
                Some(WINDOW),
            )],
            vec![(
                "{project:p1}:tokens".to_owned(),
                (-100.0_f64).to_bits(),
                Some(WINDOW),
            )],
            vec![(
                "{project:p2}:tokens".to_owned(),
                (-260.0_f64).to_bits(),
                Some(WINDOW),
            )],
        ]
    );
    assert!(fixture.usage.increments().is_empty());
    assert!(stash_state(&slot).reservation_released);
}

#[tokio::test]
async fn applies_a_windowed_project_refund_reservation_aware() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.itpm_reserved_tokens = 200;
        state.itpm_reserved_scopes = scopes(&[("project", "p1")]);
        state.itpm_reserved_windows.insert(window_identity(
            "{project:p1}:tokens",
            ReservationBackend::Redis,
        ));
    });
    let usage = token_usage(250, 10, 260);
    let event = FailureEvent {
        stash: &slot,
        call_id: None,
        combined_usage: Some(&usage),
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().log_failure_event(&event).await;
    assert_eq!(
        operation_tuples(&fixture.usage.increments()),
        vec![(
            "{project:p1}:tokens".to_owned(),
            50.0_f64.to_bits(),
            Some(WINDOW),
        )]
    );
    assert!(fixture.usage.pipelines().is_empty());
    assert!(stash_state(&slot).reservation_released);
}

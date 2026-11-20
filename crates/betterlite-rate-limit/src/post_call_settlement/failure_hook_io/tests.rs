#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::limiter_records::ReservationBackend;
use crate::post_call_settlement::failure_hook::FailureHook;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, WINDOW, install, operation_tuples, pipeline_tuples, scopes, stash_state,
    token_usage, window_identity,
};
use crate::request_stash::RequestStashSlot;

#[tokio::test]
async fn refunds_both_project_reservations_through_the_pipeline() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.itpm_reserved_tokens = 200;
        state.itpm_reserved_scopes = scopes(&[("project", "p1")]);
        state.otpm_reserved_tokens = 300;
        state.otpm_reserved_scopes = scopes(&[("project", "p2")]);
    });
    let usage = token_usage(80, 20, 100);
    let event = FailureHook {
        stash: &slot,
        combined_usage: Some(&usage),
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().post_call_failure_hook(&event).await;
    assert_eq!(
        pipeline_tuples(&fixture.usage.pipelines()),
        vec![
            vec![(
                "{project:p1}:tokens".to_owned(),
                (-120.0_f64).to_bits(),
                Some(WINDOW),
            )],
            vec![(
                "{project:p2}:tokens".to_owned(),
                (-280.0_f64).to_bits(),
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
        state.itpm_reserved_tokens = 100;
        state.itpm_reserved_scopes = scopes(&[("project", "p1")]);
        state.itpm_reserved_windows.insert(window_identity(
            "{project:p1}:tokens",
            ReservationBackend::Redis,
        ));
    });
    let usage = token_usage(150, 10, 160);
    let event = FailureHook {
        stash: &slot,
        combined_usage: Some(&usage),
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().post_call_failure_hook(&event).await;
    assert_eq!(
        operation_tuples(&fixture.usage.increments()),
        vec![(
            "{project:p1}:tokens".to_owned(),
            50.0_f64.to_bits(),
            Some(WINDOW),
        )]
    );
    assert!(fixture.usage.pipelines().is_empty());
}

#[tokio::test]
async fn skips_project_refunds_when_the_flag_is_set() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reservation_released = true;
        state.itpm_reserved_tokens = 200;
        state.itpm_reserved_scopes = scopes(&[("project", "p1")]);
        state.otpm_reserved_tokens = 300;
        state.otpm_reserved_scopes = scopes(&[("project", "p2")]);
    });
    let event = FailureHook {
        stash: &slot,
        combined_usage: None,
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    fixture.settlement().post_call_failure_hook(&event).await;
    assert!(fixture.usage.pipelines().is_empty());
    assert!(fixture.usage.increments().is_empty());
}

#![cfg(test)]

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::limiter_records::ReservationBackend;
use crate::post_call_settlement::failure_hook::FailureHook;
use crate::post_call_settlement::tests::support::{
    SettlementFixture, WINDOW, install, operation_tuples, pipeline_tuples, scopes, stash_state,
    window_identity,
};
use crate::request_stash::RequestStashSlot;
use crate::usage_resolution::Usage;

#[tokio::test]
async fn applies_the_combined_refund_and_each_project_side_on_its_own_path() {
    let fixture = SettlementFixture::default();
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
        state.itpm_reserved_tokens = 200;
        state.itpm_reserved_scopes = scopes(&[("project", "p1")]);
        state.itpm_reserved_windows.insert(window_identity(
            "{project:p1}:tokens",
            ReservationBackend::Redis,
        ));
        state.otpm_reserved_tokens = 300;
        state.otpm_reserved_scopes = scopes(&[("project", "p2")]);
    });
    let usage = Usage {
        prompt_tokens: 250,
        completion_tokens: 10,
        total_tokens: 260,
        ..Usage::default()
    };
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
                "{user:alice}:tokens".to_owned(),
                (-740.0_f64).to_bits(),
                Some(WINDOW),
            )],
            vec![(
                "{project:p2}:tokens".to_owned(),
                (-280.0_f64).to_bits(),
                Some(WINDOW),
            )],
        ]
    );
    assert_eq!(
        operation_tuples(&fixture.usage.increments()),
        vec![(
            "{project:p1}:tokens".to_owned(),
            50.0_f64.to_bits(),
            Some(WINDOW),
        )]
    );
    assert!(stash_state(&slot).reservation_released);
}

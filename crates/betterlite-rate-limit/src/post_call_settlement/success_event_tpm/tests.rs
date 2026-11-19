#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::post_call_settlement::success_event::SuccessCallback;
use crate::post_call_settlement::tests::support::{
    RerankIo, SettlementFixture, SuccessFixture, WINDOW, install, pipeline_tuples, scopes,
};
use crate::usage_resolution::Usage;

#[tokio::test]
async fn settles_a_reserved_scope_at_the_actual_usage() {
    let fixture = SettlementFixture::default();
    let mut success = SuccessFixture {
        usage: Some(Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            ..Usage::default()
        }),
        ..SuccessFixture::default()
    };
    success.logging.user_api_key_user_id = Some("alice".to_owned());
    install(&success.slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
    });
    fixture
        .settlement()
        .log_success_event(&SuccessCallback {
            event: success.event(),
            io: RerankIo::default().reconciliation(),
        })
        .await;
    assert_eq!(
        pipeline_tuples(&fixture.usage.pipelines()),
        vec![vec![(
            "{user:alice}:tokens".to_owned(),
            (-850.0_f64).to_bits(),
            Some(WINDOW),
        )]]
    );
    assert!(fixture.usage.increments().is_empty());
}

#[tokio::test]
async fn charges_an_unreserved_scope_the_full_usage() {
    let fixture = SettlementFixture::default();
    let mut success = SuccessFixture {
        usage: Some(Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            ..Usage::default()
        }),
        ..SuccessFixture::default()
    };
    success.logging.user_api_key_user_id = Some("alice".to_owned());
    install(&success.slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("team", "t1")]);
    });
    fixture
        .settlement()
        .log_success_event(&SuccessCallback {
            event: success.event(),
            io: RerankIo::default().reconciliation(),
        })
        .await;
    assert_eq!(
        pipeline_tuples(&fixture.usage.pipelines()),
        vec![vec![(
            "{user:alice}:tokens".to_owned(),
            150.0_f64.to_bits(),
            Some(WINDOW),
        )]]
    );
}

#[tokio::test]
async fn writes_no_operations_for_an_exempt_internal_call() {
    let fixture = SettlementFixture::default();
    let mut success = SuccessFixture {
        usage: Some(Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            ..Usage::default()
        }),
        ..SuccessFixture::default()
    };
    success.logging.user_api_key_user_id = Some("alice".to_owned());
    success.metadata.insert(
        "internal_call_origin".to_owned(),
        Json::Text("router".to_owned()),
    );
    install(&success.slot, |state| {
        state.reserved_tokens = 1000;
        state.reserved_scopes = scopes(&[("user", "alice")]);
    });
    fixture
        .settlement()
        .log_success_event(&SuccessCallback {
            event: success.event(),
            io: RerankIo::default().reconciliation(),
        })
        .await;
    assert!(fixture.usage.pipelines().is_empty());
    assert!(fixture.usage.increments().is_empty());
}

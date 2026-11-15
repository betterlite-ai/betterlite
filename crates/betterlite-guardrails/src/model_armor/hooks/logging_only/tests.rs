#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::model_armor::hooks::error_handling::HookError;
use crate::model_armor::hooks::tests::support::{
    API_ERROR, BLOCKED, CLEAN, Recorder, armor, armor_with, blocked, bucket, field, no_texts,
    offline_armor, prompt_data, records, settings_for, text, texts, value,
};
use crate::model_armor::transport::error::TransportError;

#[tokio::test]
async fn logging_only_returns_the_inputs_and_records_a_clean_verdict() {
    let fixture = armor_with(
        vec![(StatusCode::OK, CLEAN.to_owned())],
        settings_for(&["pre_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let inputs = texts(&["hello"]);
    let returned = hooks
        .apply_guardrail(
            inputs.clone(),
            &mut request,
            GuardrailInputType::Request,
            &mut recorder.context(),
        )
        .await
        .expect("clean");
    assert_eq!(returned, inputs);
    assert_eq!(fixture.calls(), 1);
    assert!(fixture.request(0).path.ends_with(":sanitizeUserPrompt"));
    assert_eq!(fixture.request_text(0), "hello");
    let entries = records(&request, "metadata");
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(field(entry, "guardrail_response"), value(CLEAN));
    assert_eq!(field(entry, "guardrail_status"), text("success"));
    assert_eq!(field(entry, "guardrail_provider"), text("model_armor"));
    assert_eq!(field(entry, "guardrail_mode"), value(r#"["pre_call"]"#));
    assert_eq!(field(entry, "start_time"), GuardrailValue::Decimal(100.0));
    assert_eq!(field(entry, "end_time"), GuardrailValue::Decimal(100.0));
    assert_eq!(field(entry, "duration"), GuardrailValue::Decimal(0.0));
    assert!(recorder.invocation.self_recorded);
}

#[tokio::test]
async fn logging_only_blocks_a_flagged_request() {
    let fixture = armor_with(
        vec![(StatusCode::OK, BLOCKED.to_owned())],
        settings_for(&["pre_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let error = hooks
        .apply_guardrail(
            texts(&["hello"]),
            &mut request,
            GuardrailInputType::Request,
            &mut recorder.context(),
        )
        .await
        .expect_err("blocked");
    let (status, detail) = blocked(error);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text("Content blocked by Model Armor"))
    );
    let entries = records(&request, "metadata");
    assert_eq!(
        field(&entries[0], "guardrail_status"),
        text("guardrail_flagged")
    );
    assert!(
        bucket(&request, "metadata")
            .get("applied_guardrails")
            .is_none()
    );
}

#[tokio::test]
async fn logging_only_keeps_a_flagged_request_passing_in_logging_only() {
    let fixture = armor_with(
        vec![(StatusCode::OK, BLOCKED.to_owned())],
        settings_for(&["logging_only"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let inputs = texts(&["hello"]);
    let returned = hooks
        .apply_guardrail(
            inputs.clone(),
            &mut request,
            GuardrailInputType::Request,
            &mut recorder.context(),
        )
        .await
        .expect("logged");
    assert_eq!(returned, inputs);
    let entries = records(&request, "metadata");
    assert_eq!(
        field(&entries[0], "guardrail_status"),
        text("guardrail_flagged")
    );
}

#[tokio::test]
async fn logging_only_uses_the_response_message_for_response_inputs() {
    let fixture = armor_with(
        vec![(StatusCode::OK, BLOCKED.to_owned())],
        settings_for(&["pre_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let error = hooks
        .apply_guardrail(
            texts(&["hello"]),
            &mut request,
            GuardrailInputType::Response,
            &mut recorder.context(),
        )
        .await
        .expect_err("blocked");
    let (_, detail) = blocked(error);
    assert_eq!(
        detail.get("error"),
        Some(&text("Response blocked by Model Armor"))
    );
    assert!(fixture.request(0).path.ends_with(":sanitizeModelResponse"));
}

#[tokio::test]
async fn logging_only_joins_the_non_empty_texts() {
    let fixture = armor_with(
        vec![(StatusCode::OK, CLEAN.to_owned())],
        settings_for(&["pre_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let inputs = texts(&["first", "", "second"]);
    let returned = hooks
        .apply_guardrail(
            inputs.clone(),
            &mut request,
            GuardrailInputType::Request,
            &mut recorder.context(),
        )
        .await
        .expect("clean");
    assert_eq!(returned, inputs);
    assert_eq!(fixture.request_text(0), "first\nsecond");
}

#[rstest]
#[case::absent(no_texts())]
#[case::empty(texts(&[]))]
#[tokio::test]
async fn logging_only_skips_the_request_without_texts(
    #[case] inputs: crate::guardrail_base::inputs::GuardrailApiInputs,
) {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let returned = hooks
        .apply_guardrail(
            inputs.clone(),
            &mut request,
            GuardrailInputType::Request,
            &mut recorder.context(),
        )
        .await
        .expect("empty");
    assert_eq!(returned, inputs);
    assert_eq!(fixture.calls(), 0);
    let entries = records(&request, "metadata");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        field(&entries[0], "guardrail_response"),
        GuardrailValue::Object(BTreeMap::new())
    );
}

#[tokio::test]
async fn logging_only_records_an_api_failure_without_blocking() {
    let fixture = armor_with(
        vec![(StatusCode::BAD_REQUEST, API_ERROR.to_owned())],
        settings_for(&["pre_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let inputs = texts(&["hello"]);
    let returned = hooks
        .apply_guardrail(
            inputs.clone(),
            &mut request,
            GuardrailInputType::Request,
            &mut recorder.context(),
        )
        .await
        .expect("logged");
    assert_eq!(returned, inputs);
    let entries = records(&request, "metadata");
    assert_eq!(
        field(&entries[0], "guardrail_status"),
        text("guardrail_failed_to_respond")
    );
    assert_eq!(
        field(&entries[0], "guardrail_response"),
        text(r#"Model Armor API error (upstream 400): {"error":"bad"}"#)
    );
}

#[tokio::test]
async fn logging_only_records_a_connection_failure_without_blocking() {
    let fixture = offline_armor().await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let inputs = texts(&["hello"]);
    let returned = hooks
        .apply_guardrail(
            inputs.clone(),
            &mut request,
            GuardrailInputType::Request,
            &mut recorder.context(),
        )
        .await
        .expect("logged");
    assert_eq!(returned, inputs);
    let entries = records(&request, "metadata");
    assert_eq!(
        field(&entries[0], "guardrail_status"),
        text("guardrail_failed_to_respond")
    );
    assert_eq!(
        field(&entries[0], "guardrail_response"),
        text("the Model Armor request failed")
    );
}

#[tokio::test]
async fn logging_only_propagates_a_response_that_is_not_json() {
    let fixture = armor_with(
        vec![(StatusCode::OK, "not json".to_owned())],
        settings_for(&["pre_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let error = hooks
        .apply_guardrail(
            texts(&["hello"]),
            &mut request,
            GuardrailInputType::Request,
            &mut recorder.context(),
        )
        .await
        .expect_err("transport");
    assert!(matches!(
        error,
        HookError::Transport(TransportError::ResponseJson(_))
    ));
    assert!(request.get("metadata").is_none());
}

#[tokio::test]
async fn logging_only_scans_response_inputs_through_the_model_response_endpoint() {
    let fixture = armor_with(
        vec![(StatusCode::OK, CLEAN.to_owned())],
        settings_for(&["pre_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    hooks
        .apply_guardrail(
            texts(&["hello"]),
            &mut request,
            GuardrailInputType::Response,
            &mut recorder.context(),
        )
        .await
        .expect("clean");
    assert!(fixture.request(0).path.ends_with(":sanitizeModelResponse"));
    assert_eq!(fixture.request_text(0), "hello");
}

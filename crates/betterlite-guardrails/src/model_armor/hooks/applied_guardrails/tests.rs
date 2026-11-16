#![cfg(test)]

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::record;
use crate::model_armor::hooks::tests::support::{
    BLOCKED, NAME, Recorder, armor, blocked, bucket, data, prompt_data, text,
};

#[test]
fn applied_guardrails_records_the_name_once() {
    let mut request = prompt_data();
    record(&mut request, Some(NAME));
    record(&mut request, Some(NAME));
    assert_eq!(
        bucket(&request, "metadata").get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![text(NAME)]))
    );
}

#[test]
fn applied_guardrails_without_a_name_leaves_the_data_alone() {
    let mut request = prompt_data();
    let before = request.clone();
    record(&mut request, None);
    assert_eq!(request, before);
}

#[test]
fn applied_guardrails_replaces_a_value_that_is_not_a_list() {
    let mut request = data(r#"{"metadata":{"applied_guardrails":"armor"}}"#);
    record(&mut request, Some(NAME));
    assert_eq!(
        bucket(&request, "metadata").get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![text(NAME)]))
    );
}

#[test]
fn applied_guardrails_keeps_the_existing_names_and_bucket_keys() {
    let mut request = data(r#"{"metadata":{"applied_guardrails":["first"],"caller":"kept"}}"#);
    record(&mut request, Some(NAME));
    assert_eq!(
        bucket(&request, "metadata").get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![text("first"), text(NAME)]))
    );
    assert_eq!(
        bucket(&request, "metadata").get("caller"),
        Some(&text("kept"))
    );
}

#[test]
fn applied_guardrails_uses_the_litellm_metadata_bucket_when_present() {
    let mut request = data(r#"{"litellm_metadata":{},"metadata":{}}"#);
    record(&mut request, Some(NAME));
    assert_eq!(
        bucket(&request, "litellm_metadata").get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![text(NAME)]))
    );
    assert!(bucket(&request, "metadata").is_empty());
}

#[tokio::test]
async fn applied_guardrails_are_recorded_before_a_block() {
    let fixture = armor(vec![(StatusCode::OK, BLOCKED.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    let error = hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect_err("blocked");
    blocked(error);
    assert_eq!(
        bucket(&request, "metadata").get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![text(NAME)]))
    );
}

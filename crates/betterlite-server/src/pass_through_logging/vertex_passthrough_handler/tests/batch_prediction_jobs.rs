use std::sync::Arc;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::support::{RecordingRegistry, fixture, handler, handler_with_batch, input, items};
use super::{BATCH_BODY, BATCH_ID, BATCH_MODEL_PATH, BATCH_ROUTE, CREATED};
use crate::pass_through_logging::anthropic_batch::unified_object_id;
use crate::pass_through_logging::gemini_passthrough_logging::tests::support::{
    FakeCost, as_text, field, integer, json, kwarg, kwargs, number_text, result_json, text,
};
use crate::pass_through_logging::vertex_passthrough_handler::VertexPassthroughError;

#[test]
fn batch_maps_the_input_and_returns_the_record() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(BATCH_ROUTE, BATCH_BODY);
    fixture.kwargs = kwargs(&[(
        "litellm_params",
        json(r#"{"metadata":{"user_api_key_user_id":"user-1"}}"#),
    )]);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the batch handler never fails");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "id"), "resp-1");
    assert_eq!(text(&result, "object"), "batch_prediction_job");
    assert_eq!(text(&result, "model"), BATCH_MODEL_PATH);
    assert_eq!(integer(&result, "created"), CREATED);
    let choice = &items(field(&result, "choices"))[0];
    assert_eq!(text(choice, "finish_reason"), "stop");
    let specifics = field(field(choice, "message"), "provider_specific_fields");
    assert_eq!(text(specifics, "batch_job_id"), BATCH_ID);
    assert_eq!(text(specifics, "batch_job_state"), "JOB_STATE_PENDING");
}

#[test]
fn batch_writes_the_kwargs() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(BATCH_ROUTE, BATCH_BODY);
    fixture.kwargs = kwargs(&[(
        "litellm_params",
        json(r#"{"metadata":{"user_api_key_user_id":"user-1"}}"#),
    )]);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the batch handler never fails");
    assert_eq!(as_text(kwarg(&outcome, "model")), BATCH_MODEL_PATH);
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "0");
    assert_eq!(as_text(kwarg(&outcome, "batch_id")), BATCH_ID);
    let unified = unified_object_id("gemini-2.5-flash", BATCH_ID);
    assert_eq!(
        as_text(kwarg(&outcome, "unified_object_id")),
        unified.as_str()
    );
    assert_eq!(
        as_text(kwarg(&outcome, "batch_job_state")),
        "JOB_STATE_PENDING"
    );
    assert_eq!(
        outcome.kwargs.entries.get("litellm_params"),
        Some(&json(r#"{"metadata":{"user_api_key_user_id":"user-1"}}"#))
    );
}

#[test]
fn batch_writes_the_call_log() {
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(BATCH_ROUTE, BATCH_BODY);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the batch handler never fails");
    assert_eq!(as_text(kwarg(&outcome, "model")), BATCH_MODEL_PATH);
    assert_eq!(fixture.call_log.model, BATCH_MODEL_PATH);
    let details = &fixture.call_log.model_call_details;
    assert_eq!(
        details.get("model"),
        Some(&Json::Text(BATCH_MODEL_PATH.to_owned()))
    );
    assert_eq!(
        details.get("batch_id"),
        Some(&Json::Text(BATCH_ID.to_owned()))
    );
    let recorded = details.get("response_cost").expect("the cost is written");
    assert_eq!(number_text(recorded), "0");
    assert!(cost.calls().is_empty());
}

#[test]
fn batch_missing_metadata_counts_as_empty() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(BATCH_ROUTE, BATCH_BODY);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("missing metadata is empty");
    assert_eq!(
        as_text(kwarg(&outcome, "batch_job_state")),
        "JOB_STATE_PENDING"
    );
}

#[test]
fn batch_null_litellm_params_counts_as_empty() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(BATCH_ROUTE, BATCH_BODY);
    fixture.kwargs = kwargs(&[("litellm_params", Json::Null)]);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("a null litellm_params is empty");
    assert_eq!(
        as_text(kwarg(&outcome, "batch_job_state")),
        "JOB_STATE_PENDING"
    );
}

#[test]
fn batch_list_metadata_is_an_error() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(BATCH_ROUTE, BATCH_BODY);
    fixture.kwargs = kwargs(&[("litellm_params", json(r#"{"metadata":[1]}"#))]);
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("metadata must be an object");
    assert!(matches!(
        error,
        VertexPassthroughError::Detail { field: "metadata" }
    ));
}

#[test]
fn batch_list_litellm_params_is_an_error() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(BATCH_ROUTE, BATCH_BODY);
    fixture.kwargs = kwargs(&[("litellm_params", json("[1]"))]);
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("litellm_params must be an object");
    assert!(matches!(
        error,
        VertexPassthroughError::Detail {
            field: "litellm_params"
        }
    ));
}

#[test]
fn batch_invalid_body_returns_the_failed_record() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(BATCH_ROUTE, "{oops");
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the batch handler never fails");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "model"), "vertex_ai_batch");
    assert_eq!(
        as_text(kwarg(&outcome, "batch_job_state")),
        "JOB_STATE_FAILED"
    );
}

#[tokio::test]
async fn batch_request_metadata_reaches_the_managed_registry() {
    let registry = Arc::new(RecordingRegistry::default());
    let passthrough = handler_with_batch(FakeCost::priced(Decimal::new(125, 2)), &registry);
    let mut fixture = fixture(BATCH_ROUTE, BATCH_BODY);
    fixture.kwargs = kwargs(&[(
        "litellm_params",
        json(r#"{"metadata":{"user_api_key_user_id":"user-1"}}"#),
    )]);
    let outcome = passthrough.vertex_passthrough_handler(input(&mut fixture));
    outcome.expect("the batch handler never fails");
    let registrations = registry.registrations();
    assert_eq!(registrations.len(), 1);
    let user_id = registrations[0].registrant.user_id.as_deref();
    assert_eq!(user_id, Some("user-1"));
}

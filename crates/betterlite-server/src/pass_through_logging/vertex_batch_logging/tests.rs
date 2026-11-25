#![cfg(test)]

mod support;

use std::sync::{Arc, PoisonError};

use betterlite_auth::user_roles::UserRole;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchStatus;
use chrono::DateTime;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use serde_json::Number;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::task::yield_now;

use self::support::{
    BATCH_ID, COLLECTION_ROUTE, Call, ID_SCOPED_ROUTE, MODEL, MODEL_PATH, RESPONSE_ID, Router,
    START, capture_logs, fields, job_body, logging, message, metadata, next_registration, registry,
    router, success_body,
};
use super::{VertexBatchState, extract_model_name_from_vertex_path};
use crate::pass_through_logging::anthropic_batch::{
    RegistryError, RouterModelIds, unified_object_id,
};

const GEMINI_UNIFIED: &str = "bGl0ZWxsbV9wcm94eTttb2RlbF9pZDpnZW1pbmktMi41LWZsYXNoO2xsbV9iYXRjaF9pZDozODE0ODg5NDIzNzQ5Nzc1MzYw";

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn router_port(recording: &Arc<Router>) -> Option<Arc<dyn RouterModelIds>> {
    Some(Arc::<Router>::clone(recording))
}

#[test]
fn batch_success_record_is_pending_with_zero_cost() {
    let (logged, _) = Call::new(200, &success_body(), COLLECTION_ROUTE).run(&logging(None, None));
    let result = &logged.result;
    assert_eq!(result.id, RESPONSE_ID);
    assert_eq!(result.model, MODEL_PATH);
    assert_eq!(result.object, "batch_prediction_job");
    assert_eq!(result.created, START);
    assert_eq!(result.choices.len(), 1);
    let choice = result.choices.first().expect("one choice");
    assert_eq!(
        (choice.finish_reason.as_str(), choice.index),
        ("stop", 0_u32)
    );
    let content = &choice.message;
    assert_eq!(content.role, "assistant");
    assert_eq!(
        (&content.tool_calls, &content.function_call),
        (&None, &None)
    );
    assert_eq!(
        content.content,
        format!(
            "Batch prediction job {BATCH_ID} created and is pending. Status will be updated when the batch completes."
        )
    );
    let (batch, state, unified) = (
        text(BATCH_ID),
        text("JOB_STATE_PENDING"),
        text(GEMINI_UNIFIED),
    );
    let expected = vec![
        ("batch_job_id", &batch),
        ("batch_job_state", &state),
        ("unified_object_id", &unified),
    ];
    assert_eq!(fields(&logged), expected);
    let kwargs = &logged.kwargs;
    assert_eq!(kwargs.response_cost, Decimal::ZERO);
    assert_eq!(kwargs.model, MODEL_PATH);
    assert_eq!(kwargs.batch_id.as_deref(), Some(BATCH_ID));
    assert_eq!(kwargs.unified_object_id.as_deref(), Some(GEMINI_UNIFIED));
    assert_eq!(kwargs.batch_job_state, VertexBatchState::JobStatePending);
}

#[test]
fn batch_unified_object_id_comes_from_the_shared_function() {
    assert_eq!(unified_object_id(MODEL, BATCH_ID), GEMINI_UNIFIED);
    let mut call = Call::new(200, &success_body(), ID_SCOPED_ROUTE);
    call.request_metadata = metadata(r#"{"user_api_key_user_id":"user-1"}"#);
    let (logged, _) = call.run(&logging(None, None));
    assert_eq!(logged.kwargs.request_metadata, call.request_metadata);
    assert_eq!(
        logged.kwargs.batch_job_state,
        VertexBatchState::JobStatePending
    );
}

#[test]
fn batch_success_writes_the_call_log() {
    let (_, call_log) = Call::new(200, &success_body(), ID_SCOPED_ROUTE).run(&logging(None, None));
    assert_eq!(call_log.model, MODEL_PATH);
    let details = &call_log.model_call_details;
    assert_eq!(details.get("model"), Some(&text(MODEL_PATH)));
    assert_eq!(
        details.get("response_cost"),
        Some(&Json::Number(Number::from(0_u8)))
    );
    assert_eq!(details.get("batch_id"), Some(&text(BATCH_ID)));
    assert_eq!(details.get("custom_llm_provider"), None);
}

#[test]
fn batch_created_is_start_time_in_whole_seconds() {
    let mut call = Call::new(200, &success_body(), ID_SCOPED_ROUTE);
    call.start_time = DateTime::from_timestamp(1_717_243_200, 900_000_000).expect("a valid start");
    let (logged, _) = call.run(&logging(None, None));
    assert_eq!(logged.result.created, 1_717_243_200_i64);
}

#[rstest]
#[case::empty_name("", "")]
#[case::bare_name("job-7", "job-7")]
#[case::trailing_slash("projects/p/batchPredictionJobs/", "")]
fn batch_id_is_the_last_name_component(#[case] name: &str, #[case] batch_id: &str) {
    let body = job_body(name, Some(r#""gemini-2.5-flash""#));
    let (logged, call_log) = Call::new(200, &body, ID_SCOPED_ROUTE).run(&logging(None, None));
    assert_eq!(logged.kwargs.batch_id.as_deref(), Some(batch_id));
    assert_eq!(
        call_log.model_call_details.get("batch_id"),
        Some(&text(batch_id))
    );
    assert_eq!(
        logged.kwargs.unified_object_id,
        Some(unified_object_id(MODEL, batch_id))
    );
}

#[rstest]
#[case::missing(None)]
#[case::null(Some("null"))]
fn batch_absent_model_is_unknown(#[case] model: Option<&str>) {
    let body = job_body("projects/p/batchPredictionJobs/9", model);
    let (logged, call_log) = Call::new(200, &body, ID_SCOPED_ROUTE).run(&logging(None, None));
    assert_eq!(logged.result.model, "unknown");
    assert_eq!(call_log.model, "unknown");
    assert_eq!(
        logged.kwargs.unified_object_id,
        Some(unified_object_id("unknown", "9"))
    );
}

#[rstest]
#[case::number("5", "a number")]
#[case::boolean("true", "a boolean")]
#[case::list("[]", "a list")]
#[case::object("{}", "an object")]
fn batch_non_string_model_gives_error_record(#[case] model: &str, #[case] kind: &str) {
    let body = job_body("projects/p/batchPredictionJobs/9", Some(model));
    let (logged, call_log) = Call::new(200, &body, ID_SCOPED_ROUTE).run(&logging(None, None));
    let error = format!("the batch prediction job model is {kind}, not a string");
    assert_eq!(
        fields(&logged),
        vec![
            ("batch_job_state", &text("JOB_STATE_FAILED")),
            ("error", &text(&error))
        ]
    );
    assert_eq!(
        message(&logged).content,
        format!("Error creating batch prediction job: {error}")
    );
    assert_eq!(call_log.model, "");
    assert_eq!(call_log.model_call_details.get("batch_id"), None);
}

#[rstest]
#[case::server_error(500, success_body())]
#[case::created_status(201, success_body())]
#[case::without_name(200, r#"{"state":"JOB_STATE_PENDING"}"#.to_owned())]
#[case::list_body(200, r#"["name"]"#.to_owned())]
#[case::text_body(200, r#""name""#.to_owned())]
#[case::null_body(200, "null".to_owned())]
#[tokio::test]
async fn batch_rejected_response_gives_failed_record(#[case] status: u16, #[case] body: String) {
    let (port, mut received) = registry(Ok(()));
    let (logged, call_log) =
        Call::new(status, &body, COLLECTION_ROUTE).run(&logging(None, Some(port)));
    assert_eq!(logged.result.id, RESPONSE_ID);
    assert_eq!(logged.result.model, "vertex_ai_batch");
    assert_eq!(logged.result.object, "batch_prediction_job");
    assert_eq!(logged.result.created, START);
    assert_eq!(
        message(&logged).content,
        format!("Batch prediction job creation failed. Status: {status}")
    );
    let code = Json::Number(Number::from(status));
    assert_eq!(
        fields(&logged),
        vec![
            ("batch_job_state", &text("JOB_STATE_FAILED")),
            ("status_code", &code)
        ]
    );
    let kwargs = &logged.kwargs;
    assert_eq!(
        (kwargs.response_cost, kwargs.model.as_str()),
        (Decimal::ZERO, "vertex_ai_batch")
    );
    assert_eq!(
        (&kwargs.batch_id, &kwargs.unified_object_id),
        (&None, &None)
    );
    assert_eq!(kwargs.batch_job_state, VertexBatchState::JobStateFailed);
    assert_eq!(call_log.model, "");
    assert_eq!(call_log.model_call_details.get("model"), Some(&text("")));
    assert_eq!(call_log.model_call_details.get("response_cost"), None);
    yield_now().await;
    assert!(matches!(received.try_recv(), Err(TryRecvError::Empty)));
}

#[rstest]
#[case::not_json(
    500,
    "not json",
    "the batch prediction job response body is not valid JSON"
)]
#[case::not_json_on_200(200, "{", "the batch prediction job response body is not valid JSON")]
#[case::name_not_text(
    200,
    r#"{"name":5}"#,
    "the batch prediction job response is not a Vertex AI batch job"
)]
#[case::no_create_time(
    200,
    r#"{"name":"a/b"}"#,
    "the batch prediction job response cannot become a batch record"
)]
fn batch_failure_gives_error_record(#[case] status: u16, #[case] body: &str, #[case] error: &str) {
    let (logs, _guard) = capture_logs();
    let (logged, call_log) = Call::new(status, body, ID_SCOPED_ROUTE).run(&logging(None, None));
    assert_eq!(logged.result.model, "vertex_ai_batch");
    assert_eq!(
        logged.kwargs.batch_job_state,
        VertexBatchState::JobStateFailed
    );
    assert_eq!(logged.kwargs.batch_id, None);
    let content = &message(&logged).content;
    assert!(content.starts_with(&format!("Error creating batch prediction job: {error}")));
    let names: Vec<&str> = fields(&logged).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["batch_job_state", "error"]);
    assert_eq!(call_log.model, "");
    assert!(
        logs.text()
            .contains(&format!("Error in batch_prediction_jobs_handler: {error}"))
    );
}

#[test]
fn batch_router_absent_uses_extracted_name() {
    let (logs, _guard) = capture_logs();
    let handler = logging(None, None);
    assert_eq!(handler.get_actual_model_id_from_router(MODEL_PATH), MODEL);
    assert!(
        logs.text()
            .contains("Router not available, using constructed model name: gemini-2.5-flash")
    );
}

#[test]
fn batch_router_empty_uses_extracted_name() {
    let (logs, _guard) = capture_logs();
    let recording = router(&[]);
    let handler = logging(router_port(&recording), None);
    assert_eq!(handler.get_actual_model_id_from_router(MODEL_PATH), MODEL);
    assert!(
        logs.text()
            .contains("Model not found in router, using constructed name: gemini-2.5-flash")
    );
    let asked = recording
        .asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(asked, [MODEL]);
}

#[test]
fn batch_router_found_uses_first_id() {
    let (logs, _guard) = capture_logs();
    let recording = router(&["deployment-1", "deployment-2"]);
    let handler = logging(router_port(&recording), None);
    let (logged, _) = Call::new(200, &success_body(), ID_SCOPED_ROUTE).run(&handler);
    assert_eq!(
        logged.kwargs.unified_object_id,
        Some(unified_object_id("deployment-1", BATCH_ID))
    );
    assert_eq!(logged.result.model, MODEL_PATH);
    assert!(
        logs.text()
            .contains("Found model ID in router: deployment-1")
    );
    let asked = recording
        .asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(asked, [MODEL]);
}

#[rstest]
#[case::publishers("publishers/google/models/gemini-2.5-flash", "gemini-2.5-flash")]
#[case::projects("projects/P/locations/L/models/123", "123")]
#[case::last_models("projects/P/models/a/models/b", "b")]
#[case::plain("gemini-2.5-flash", "gemini-2.5-flash")]
#[case::models_only("models/x", "models/x")]
#[case::publishers_without_models("publishers/google/gemini", "publishers/google/gemini")]
#[case::projects_without_models("projects/P/endpoints/1", "projects/P/endpoints/1")]
#[case::empty("", "")]
fn batch_extract_model_name_from_vertex_path(#[case] path: &str, #[case] expected: &str) {
    assert_eq!(extract_model_name_from_vertex_path(path), expected);
}

#[tokio::test]
async fn batch_collection_route_registers_with_attribution() {
    let (logs, _guard) = capture_logs();
    let (port, mut received) = registry(Ok(()));
    let mut call = Call::new(200, &success_body(), COLLECTION_ROUTE);
    call.request_metadata = metadata(
        r#"{"user_api_key_user_id":"user-1","user_api_key":"sk-key","user_api_key_team_id":"team-1","tags":["a","b"]}"#,
    );
    let (logged, _) = call.run(&logging(None, Some(port)));
    let registration = next_registration(&mut received).await;
    assert_eq!(registration.unified_object_id, GEMINI_UNIFIED);
    assert_eq!(registration.model_object_id, BATCH_ID);
    assert_eq!(registration.batch_object.id, BATCH_ID);
    assert_eq!(registration.batch_object.status, BatchStatus::Validating);
    assert_eq!(registration.batch_object.created_at, 1_704_067_200_i64);
    assert_eq!(
        registration.batch_object.input_file_id,
        "gs://bucket/in.jsonl"
    );
    assert_eq!(registration.file_purpose, "batch");
    assert_eq!(
        (
            registration.persist_attribution,
            registration.create_if_missing
        ),
        (true, true)
    );
    assert_eq!(
        registration.request_tags,
        Some(vec!["a".to_owned(), "b".to_owned()])
    );
    let registrant = &registration.registrant;
    assert_eq!(registrant.user_id.as_deref(), Some("user-1"));
    assert_eq!(
        registrant
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("sk-key".to_owned())
    );
    assert_eq!(registrant.team_id.as_deref(), Some("team-1"));
    assert_eq!(
        (registrant.user_role, registrant.spend),
        (UserRole::Customer, Decimal::ZERO)
    );
    let empty = (
        registrant.models.len(),
        registrant.metadata.len(),
        registrant.permissions.len(),
    );
    assert_eq!(empty, (0, 0, 0));
    assert_eq!(
        (
            registrant.model_max_budget.len(),
            registrant.model_spend.len()
        ),
        (0, 0)
    );
    let unset = (
        registrant.team_alias.as_deref(),
        registrant.user_email.as_deref(),
        registrant.max_budget,
        registrant.tpm_limit,
        registrant.rpm_limit,
    );
    assert_eq!(unset, (None, None, None, None, None));
    let rest = (
        registrant.budget_duration.as_deref(),
        registrant.budget_reset_at,
        registrant.max_parallel_requests,
        registrant.allowed_model_region.as_deref(),
        registrant.key_alias.as_deref(),
    );
    assert_eq!(rest, (None, None, None, None, None));
    logs.wait_for("Stored Vertex AI batch managed object").await;
    assert!(logs.text().contains(&format!("Stored Vertex AI batch managed object with unified_object_id={GEMINI_UNIFIED}, batch_id={BATCH_ID}")));
    assert_eq!(
        logged.kwargs.batch_job_state,
        VertexBatchState::JobStatePending
    );
}

#[tokio::test]
async fn batch_id_scoped_route_registers_without_attribution() {
    let (logs, _guard) = capture_logs();
    let (port, mut received) = registry(Err(RegistryError {
        message: "row missing".to_owned(),
    }));
    let (logged, _) =
        Call::new(200, &success_body(), ID_SCOPED_ROUTE).run(&logging(None, Some(port)));
    let registration = next_registration(&mut received).await;
    assert_eq!(
        (
            registration.persist_attribution,
            registration.create_if_missing
        ),
        (false, false)
    );
    assert_eq!(registration.model_object_id, BATCH_ID);
    logs.wait_for("row missing").await;
    let text_logs = logs.text();
    assert!(text_logs.contains("Failed to store Vertex AI batch managed object"));
    assert!(text_logs.contains("its status and output file may be stale: row missing"));
    assert_eq!(
        logged.kwargs.batch_job_state,
        VertexBatchState::JobStatePending
    );
}

#[tokio::test]
async fn batch_collection_route_failure_says_cost_is_untracked() {
    let (logs, _guard) = capture_logs();
    let (port, mut received) = registry(Err(RegistryError {
        message: "down".to_owned(),
    }));
    let route = "/vertex_ai/v1/projects/p/locations/l/batchPredictionJobs/?alt=json";
    drop(Call::new(200, &success_body(), route).run(&logging(None, Some(port))));
    let registration = next_registration(&mut received).await;
    assert_eq!(
        (
            registration.persist_attribution,
            registration.create_if_missing
        ),
        (true, true)
    );
    logs.wait_for("down").await;
    assert!(logs.text().contains("its cost will not be tracked: down"));
}

#[rstest]
#[case::no_metadata("{}", Some("default-user"), None)]
#[case::null_user(r#"{"user_api_key_user_id":null}"#, None, None)]
#[case::key_tags(r#"{"user_api_key_auth_metadata":{"tags":["k"]}}"#, Some("default-user"), Some(vec!["k".to_owned()]))]
#[tokio::test]
async fn batch_registrant_defaults(
    #[case] meta: &str,
    #[case] user_id: Option<&str>,
    #[case] tags: Option<Vec<String>>,
) {
    let (port, mut received) = registry(Ok(()));
    let mut call = Call::new(200, &success_body(), COLLECTION_ROUTE);
    call.request_metadata = metadata(meta);
    drop(call.run(&logging(None, Some(port))));
    let registration = next_registration(&mut received).await;
    assert_eq!(registration.registrant.user_id.as_deref(), user_id);
    assert!(registration.registrant.api_key.is_none());
    assert_eq!(registration.registrant.team_id, None);
    assert_eq!(registration.request_tags, tags);
}

#[test]
fn batch_missing_registry_only_warns() {
    let (logs, _guard) = capture_logs();
    let (logged, _) = Call::new(200, &success_body(), COLLECTION_ROUTE).run(&logging(None, None));
    assert_eq!(
        logged.kwargs.batch_job_state,
        VertexBatchState::JobStatePending
    );
    assert!(
        logs.text().contains(
            "Managed files hook not available, cannot store batch object for cost tracking"
        )
    );
}

#[test]
fn batch_missing_runtime_logs_and_stays_pending() {
    let (logs, _guard) = capture_logs();
    let (port, _received) = registry(Ok(()));
    let (logged, _) =
        Call::new(200, &success_body(), COLLECTION_ROUTE).run(&logging(None, Some(port)));
    assert_eq!(
        logged.kwargs.batch_job_state,
        VertexBatchState::JobStatePending
    );
    let text_logs = logs.text();
    assert!(text_logs.contains("Error storing batch managed object: the batch managed object cannot be spawned without a Tokio runtime"));
    assert!(!text_logs.contains("Managed files hook not available"));
}

#[test]
fn batch_state_serializes_as_vertex_job_states() {
    let words = serde_json::to_string(&[
        VertexBatchState::JobStatePending,
        VertexBatchState::JobStateFailed,
    ])
    .expect("serialized");
    assert_eq!(words, r#"["JOB_STATE_PENDING","JOB_STATE_FAILED"]"#);
}

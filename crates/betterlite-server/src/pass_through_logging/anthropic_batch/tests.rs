#![cfg(test)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{BatchObjectKind, BatchStatus};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Number;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::task::yield_now;
use tokio::time::timeout;

use self::support::{
    BATCH_ID, COLLECTION_ROUTE, CountingIds, Creation, ID_SCOPED_ROUTE, MODEL, RESPONSE_ID,
    assert_registrant_fields, batch_body, batch_object, capture_logs, field, handler,
    handler_with_ids, message, metadata, provider_fields, registry_port, router, router_port,
    router_port_of, run, start_time, text,
};
use super::{BatchJobState, RegistryError, unified_object_id};

const SONNET_UNIFIED: &str = "bGl0ZWxsbV9wcm94eTttb2RlbF9pZDphbnRocm9waWMvY2xhdWRlLXNvbm5ldC00LTU7bGxtX2JhdGNoX2lkOm1zZ2JhdGNoXzE";

#[test]
fn batch_success_record_matches_litellm_shape() {
    let (logged, _) = run(&handler(None, None), &Creation::success(MODEL));
    assert_eq!(logged.result.id, RESPONSE_ID);
    assert_eq!(logged.result.model, MODEL);
    assert_eq!(logged.result.object, "batch");
    assert_eq!(logged.result.created, 1_767_225_600_i64);
    assert_eq!(logged.result.choices.len(), 1);
    let choice = logged.result.choices.first().expect("one choice");
    assert_eq!(choice.finish_reason, "stop");
    assert_eq!(choice.index, 0_u32);
    assert_eq!(choice.message.role, "assistant");
    assert_eq!(
        choice.message.content,
        format!(
            "Batch job {BATCH_ID} created and is pending. Status will be updated when the batch completes."
        )
    );
    let fields = provider_fields(&logged);
    let names: Vec<&str> = fields.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        ["batch_job_id", "batch_job_state", "unified_object_id"]
    );
    assert_eq!(text(fields, "batch_job_id"), BATCH_ID);
    assert_eq!(text(fields, "batch_job_state"), "in_progress");
    assert_eq!(
        text(fields, "unified_object_id"),
        unified_object_id("anthropic/claude-x", BATCH_ID)
    );
}

#[test]
fn batch_success_message_has_no_tool_calls() {
    let (logged, _) = run(&handler(None, None), &Creation::success(MODEL));
    let logged_message = message(&logged);
    assert_eq!(logged_message.tool_calls, None);
    assert_eq!(logged_message.function_call, None);
}

#[test]
fn batch_success_kwargs_match_litellm() {
    const META: &str = r#"{"user_api_key_user_id":"user-1"}"#;
    let creation = Creation::success(MODEL).with_metadata(META);
    let (logged, _) = run(&handler(None, None), &creation);
    assert_eq!(logged.kwargs.response_cost, Decimal::ZERO);
    assert_eq!(logged.kwargs.model, MODEL);
    assert_eq!(logged.kwargs.batch_id, Some(BATCH_ID.to_owned()));
    assert_eq!(
        logged.kwargs.unified_object_id,
        Some(unified_object_id("anthropic/claude-x", BATCH_ID))
    );
    assert_eq!(logged.kwargs.batch_job_state, BatchJobState::InProgress);
    assert_eq!(logged.kwargs.request_metadata, metadata(META));
}

#[test]
fn batch_success_updates_call_log() {
    let (logged, call_log) = run(&handler(None, None), &Creation::success(MODEL));
    assert_eq!(call_log.model, MODEL);
    assert_eq!(
        call_log.model_call_details.get("model"),
        Some(&Json::Text(MODEL.to_owned()))
    );
    assert_eq!(
        call_log.model_call_details.get("response_cost"),
        Some(&Json::Number(Number::from(0_u8)))
    );
    assert_eq!(
        call_log.model_call_details.get("batch_id"),
        Some(&Json::Text(BATCH_ID.to_owned()))
    );
    assert!(call_log.model_call_details.contains_key("litellm_params"));
    assert_eq!(logged.result.model, MODEL);
}

#[test]
fn batch_success_created_from_start_time() {
    let start = DateTime::parse_from_rfc3339("2024-06-01T12:00:00Z")
        .expect("a valid timestamp")
        .with_timezone(&Utc);
    let creation = Creation::success(MODEL).with_start_time(start);
    let (logged, _) = run(&handler(None, None), &creation);
    assert_eq!(logged.result.created, 1_717_243_200_i64);
    assert_ne!(logged.result.created, start_time().timestamp());
}

#[test]
fn batch_success_model_from_top_level_request_body() {
    let (logged, call_log) = run(&handler(None, None), &Creation::success(MODEL));
    assert_eq!(logged.result.model, MODEL);
    assert_eq!(
        text(provider_fields(&logged), "unified_object_id"),
        unified_object_id("anthropic/claude-x", BATCH_ID)
    );
    assert_eq!(call_log.model, MODEL);
}

#[test]
fn batch_success_model_from_nested_params() {
    let creation = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"requests":[{"params":{"model":"claude-nested"}}]}"#);
    let (logged, _) = run(&handler(None, None), &creation);
    assert_eq!(logged.result.model, "claude-nested");
    assert_eq!(
        text(provider_fields(&logged), "unified_object_id"),
        unified_object_id("anthropic/claude-nested", BATCH_ID)
    );
}

#[test]
fn batch_success_model_falls_back_to_unknown() {
    let no_body = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE);
    let (first, _) = run(&handler(None, None), &no_body);
    assert_eq!(first.result.model, "unknown");
    assert_eq!(
        text(provider_fields(&first), "unified_object_id"),
        unified_object_id("anthropic/unknown", BATCH_ID)
    );
    let empty_requests = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"requests":[]}"#);
    let (second, _) = run(&handler(None, None), &empty_requests);
    assert_eq!(second.result.model, "unknown");
    let scalar_request = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"requests":[5]}"#);
    let (third, _) = run(&handler(None, None), &scalar_request);
    assert_eq!(third.result.model, "unknown");
    let empty_nested = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"requests":[{"params":{"model":""}}]}"#);
    let (fourth, _) = run(&handler(None, None), &empty_nested);
    assert_eq!(fourth.result.model, "unknown");
    let non_string_nested = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"requests":[{"params":{"model":5}}]}"#);
    let (fifth, _) = run(&handler(None, None), &non_string_nested);
    assert_eq!(fifth.result.model, "unknown");
    let non_object_params = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"requests":[{"params":5}]}"#);
    let (sixth, _) = run(&handler(None, None), &non_object_params);
    assert_eq!(sixth.result.model, "unknown");
}

#[test]
fn batch_success_empty_top_level_model_falls_back_to_nested() {
    let creation = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"model":"","requests":[{"params":{"model":"nested"}}]}"#);
    let (logged, _) = run(&handler(None, None), &creation);
    assert_eq!(logged.result.model, "nested");
    let alone = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"model":""}"#);
    let (second, _) = run(&handler(None, None), &alone);
    assert_eq!(second.result.model, "unknown");
}

#[test]
fn batch_success_top_level_model_wins_over_nested() {
    let creation = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"model":"top","requests":[{"params":{"model":"nested"}}]}"#);
    let (logged, _) = run(&handler(None, None), &creation);
    assert_eq!(logged.result.model, "top");
}

#[test]
fn batch_success_non_string_top_level_model_uses_nested() {
    let creation = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"model":5,"requests":[{"params":{"model":"nested"}}]}"#);
    let (logged, _) = run(&handler(None, None), &creation);
    assert_eq!(logged.result.model, "nested");
}

#[test]
fn batch_success_unknown_model_string_uses_nested() {
    let creation = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"model":"unknown","requests":[{"params":{"model":"nested"}}]}"#);
    let (logged, _) = run(&handler(None, None), &creation);
    assert_eq!(logged.result.model, "nested");
    assert_eq!(
        text(provider_fields(&logged), "unified_object_id"),
        unified_object_id("anthropic/nested", BATCH_ID)
    );
    let alone = Creation::new(200, batch_body(BATCH_ID), ID_SCOPED_ROUTE)
        .with_request_body(r#"{"model":"unknown"}"#);
    let (second, _) = run(&handler(None, None), &alone);
    assert_eq!(second.result.model, "unknown");
}

#[test]
fn batch_success_router_id_replaces_model_in_unified_id() {
    let recording = router(&["litellm-deployment"]);
    let logging = handler(Some(router_port_of(&recording)), None);
    let (logged, _) = run(&logging, &Creation::success(MODEL));
    assert_eq!(logged.result.model, MODEL);
    assert_eq!(
        text(provider_fields(&logged), "unified_object_id"),
        unified_object_id("litellm-deployment", BATCH_ID)
    );
    assert_eq!(recording.asked(), [MODEL.to_owned()]);
}

#[test]
fn batch_success_router_absent_warns_and_prefixes() {
    let (capture, _lines, _guard) = capture_logs("");
    let (logged, _) = run(&handler(None, None), &Creation::success(MODEL));
    assert_eq!(
        text(provider_fields(&logged), "unified_object_id"),
        unified_object_id("anthropic/claude-x", BATCH_ID)
    );
    assert!(
        capture
            .contents()
            .contains("Router not available, using model name: claude-x")
    );
}

#[test]
fn batch_success_router_empty_warns_and_prefixes() {
    let (capture, _lines, _guard) = capture_logs("");
    let (logged, _) = run(
        &handler(Some(router_port(&[])), None),
        &Creation::success(MODEL),
    );
    assert_eq!(
        text(provider_fields(&logged), "unified_object_id"),
        unified_object_id("anthropic/claude-x", BATCH_ID)
    );
    assert!(
        capture
            .contents()
            .contains("Model not found in router, using model name: claude-x")
    );
}

#[test]
fn batch_success_router_found_logs_info_and_uses_first_id() {
    let recording = router(&["deployment-1", "deployment-2"]);
    let (capture, _lines, _guard) = capture_logs("");
    let logging = handler(Some(router_port_of(&recording)), None);
    let (logged, _) = run(&logging, &Creation::success(MODEL));
    assert_eq!(
        text(provider_fields(&logged), "unified_object_id"),
        unified_object_id("deployment-1", BATCH_ID)
    );
    assert!(
        capture
            .contents()
            .contains("Found model ID in router: deployment-1")
    );
    assert_eq!(recording.asked(), [MODEL.to_owned()]);
}

#[test]
fn batch_success_already_prefixed_model_keeps_one_prefix() {
    let (logged, _) = run(
        &handler(None, None),
        &Creation::success("anthropic/claude-opus-4"),
    );
    assert_eq!(
        text(provider_fields(&logged), "unified_object_id"),
        unified_object_id("anthropic/claude-opus-4", BATCH_ID)
    );
    let recording = router(&["anthropic/claude-opus-4"]);
    let logging = handler(Some(router_port_of(&recording)), None);
    let (routed, _) = run(&logging, &Creation::success("anthropic/claude-opus-4"));
    assert_eq!(
        text(provider_fields(&routed), "unified_object_id"),
        unified_object_id("anthropic/claude-opus-4", BATCH_ID)
    );
}

#[test]
fn batch_success_uses_a_fresh_response_id_per_record() {
    let logging = handler_with_ids(None, None, Arc::new(CountingIds::new()));
    let (first, _) = run(&logging, &Creation::success(MODEL));
    let (second, _) = run(&logging, &Creation::success(MODEL));
    assert_eq!(first.result.id, "id-1");
    assert_eq!(second.result.id, "id-2");
}

#[test]
fn batch_non_200_gives_failed_record() {
    let creation = Creation::new(500, batch_body(BATCH_ID), COLLECTION_ROUTE)
        .with_request_body(r#"{"model":"claude-x"}"#);
    let (logged, _) = run(&handler(None, None), &creation);
    assert_eq!(logged.result.id, RESPONSE_ID);
    assert_eq!(logged.result.model, "anthropic_batch");
    assert_eq!(logged.result.object, "batch");
    assert_eq!(logged.result.created, 1_767_225_600_i64);
    assert_eq!(
        message(&logged).content,
        "Batch job creation failed. Status: 500"
    );
    let fields = provider_fields(&logged);
    assert_eq!(fields.len(), 2);
    assert_eq!(text(fields, "batch_job_state"), "failed");
    assert_eq!(
        field(fields, "status_code"),
        &Json::Number(Number::from(500_u16))
    );
    assert_eq!(logged.kwargs.response_cost, Decimal::ZERO);
    assert_eq!(logged.kwargs.model, "anthropic_batch");
    assert_eq!(logged.kwargs.batch_id, None);
    assert_eq!(logged.kwargs.unified_object_id, None);
    assert_eq!(logged.kwargs.batch_job_state, BatchJobState::Failed);
}

#[test]
fn batch_non_200_leaves_call_log_alone() {
    let (port, mut received) = registry_port(Ok(()));
    let creation = Creation::new(500, batch_body(BATCH_ID), COLLECTION_ROUTE)
        .with_request_body(r#"{"model":"claude-x"}"#);
    let logging = handler(None, port);
    let (logged, call_log) = run(&logging, &creation);
    assert_eq!(logged.kwargs.batch_job_state, BatchJobState::Failed);
    assert_eq!(call_log.model, "");
    assert_eq!(
        call_log.model_call_details.get("model"),
        Some(&Json::Text(String::new()))
    );
    assert_eq!(call_log.model_call_details.get("batch_id"), None);
    assert_eq!(call_log.model_call_details.get("response_cost"), None);
    assert!(call_log.model_call_details.contains_key("litellm_params"));
    assert!(matches!(received.try_recv(), Err(TryRecvError::Empty)));
}

#[test]
fn batch_non_200_unparsable_body_gives_error_record() {
    let (capture, _lines, _guard) = capture_logs("");
    let creation = Creation::new(500, b"not json".to_vec(), ID_SCOPED_ROUTE);
    let (logged, call_log) = run(&handler(None, None), &creation);
    assert_eq!(logged.result.model, "anthropic_batch");
    assert_eq!(logged.kwargs.batch_job_state, BatchJobState::Failed);
    assert!(
        message(&logged)
            .content
            .starts_with("Error creating batch job: ")
    );
    assert!(
        text(provider_fields(&logged), "error")
            .starts_with("the batch response body is not valid JSON")
    );
    assert_eq!(call_log.model, "");
    assert_eq!(call_log.model_call_details.get("batch_id"), None);
    assert!(
        capture
            .contents()
            .contains("Error in batch_creation_handler: the batch response body is not valid JSON")
    );
}

#[test]
fn batch_200_without_id_gives_failed_record() {
    let creation = Creation::new(200, br#"{"object":"batch"}"#.to_vec(), ID_SCOPED_ROUTE);
    let (logged, call_log) = run(&handler(None, None), &creation);
    assert_eq!(
        message(&logged).content,
        "Batch job creation failed. Status: 200"
    );
    let fields = provider_fields(&logged);
    assert_eq!(text(fields, "batch_job_state"), "failed");
    assert_eq!(
        field(fields, "status_code"),
        &Json::Number(Number::from(200_u16))
    );
    assert_eq!(logged.kwargs.batch_job_state, BatchJobState::Failed);
    assert_eq!(call_log.model, "");
}

#[test]
fn batch_200_unsearchable_scalar_bodies_give_error_records() {
    let number = Creation::new(200, b"5".to_vec(), ID_SCOPED_ROUTE);
    let (first, _) = run(&handler(None, None), &number);
    assert!(
        text(provider_fields(&first), "error")
            .contains("is a number, which cannot be searched for an id")
    );
    assert!(
        message(&first)
            .content
            .contains("is a number, which cannot be searched for an id")
    );
    let null = Creation::new(200, b"null".to_vec(), ID_SCOPED_ROUTE);
    let (second, _) = run(&handler(None, None), &null);
    assert!(
        text(provider_fields(&second), "error")
            .contains("is null, which cannot be searched for an id")
    );
    let boolean = Creation::new(200, b"true".to_vec(), ID_SCOPED_ROUTE);
    let (third, _) = run(&handler(None, None), &boolean);
    assert!(
        text(provider_fields(&third), "error")
            .contains("is a boolean, which cannot be searched for an id")
    );
    assert_eq!(third.kwargs.batch_job_state, BatchJobState::Failed);
}

#[test]
fn batch_200_array_body_uses_membership() {
    let with_id = Creation::new(200, br#"["id"]"#.to_vec(), ID_SCOPED_ROUTE);
    let (logged, _) = run(&handler(None, None), &with_id);
    assert!(text(provider_fields(&logged), "error").contains("the body must be a JSON object"));
    let without_id = Creation::new(200, b"[1,2]".to_vec(), ID_SCOPED_ROUTE);
    let (failed, _) = run(&handler(None, None), &without_id);
    assert_eq!(
        message(&failed).content,
        "Batch job creation failed. Status: 200"
    );
}

#[test]
fn batch_200_string_body_uses_substring() {
    let with_id = Creation::new(200, br#""a batch id""#.to_vec(), ID_SCOPED_ROUTE);
    let (logged, _) = run(&handler(None, None), &with_id);
    assert!(text(provider_fields(&logged), "error").contains("the body must be a JSON object"));
    let without_id = Creation::new(200, br#""nothing""#.to_vec(), ID_SCOPED_ROUTE);
    let (failed, _) = run(&handler(None, None), &without_id);
    assert_eq!(
        message(&failed).content,
        "Batch job creation failed. Status: 200"
    );
}

#[test]
fn batch_transform_error_gives_error_record() {
    let creation = Creation::new(200, br#"{"id":true}"#.to_vec(), ID_SCOPED_ROUTE);
    let (logged, _) = run(&handler(None, None), &creation);
    assert!(text(provider_fields(&logged), "error").contains("id must be a string"));
    assert!(
        message(&logged)
            .content
            .starts_with("Error creating batch job: ")
    );
}

#[tokio::test]
async fn batch_collection_route_registers_managed_object() {
    const META: &str = r#"{"user_api_key_user_id":"user-1","user_api_key":"sk-key","user_api_key_team_id":"team-1","tags":["a","b"]}"#;
    let (port, mut received) = registry_port(Ok(()));
    let (capture, mut lines, _guard) = capture_logs("Stored Anthropic batch managed object");
    let creation = Creation::new(200, batch_body(BATCH_ID), COLLECTION_ROUTE)
        .with_request_body(r#"{"model":"claude-x"}"#)
        .with_metadata(META);
    let (logged, _) = run(&handler(None, port), &creation);
    let registration = timeout(Duration::from_secs(5), received.recv())
        .await
        .expect("a registration arrives")
        .expect("the registry records one");
    assert_eq!(
        registration.unified_object_id,
        unified_object_id("anthropic/claude-x", BATCH_ID)
    );
    assert_eq!(registration.batch_object.id, BATCH_ID);
    assert_eq!(registration.batch_object.status, BatchStatus::Validating);
    assert_eq!(registration.batch_object.object, BatchObjectKind::Batch);
    assert_eq!(registration.model_object_id, BATCH_ID);
    assert_eq!(registration.file_purpose, "batch");
    assert!(registration.persist_attribution);
    assert!(registration.create_if_missing);
    assert_eq!(
        registration.request_tags,
        Some(vec!["a".to_owned(), "b".to_owned()])
    );
    let registrant = &registration.registrant;
    assert_registrant_fields(registrant);
    timeout(Duration::from_secs(5), lines.recv())
        .await
        .expect("the done callback logs")
        .expect("a log line arrives");
    let logs = capture.contents();
    assert!(logs.contains("Stored Anthropic batch managed object with unified_object_id="));
    assert!(logs.contains(&format!("batch_id={BATCH_ID}")));
    assert_eq!(logged.kwargs.batch_id, Some(BATCH_ID.to_owned()));
}

#[tokio::test]
async fn batch_collection_route_registrant_defaults_user_id_and_preserves_null() {
    let (port, mut received) = registry_port(Ok(()));
    let collection = Creation::new(200, batch_body(BATCH_ID), COLLECTION_ROUTE);
    drop(run(&handler(None, port), &collection));
    let defaulted = timeout(Duration::from_secs(5), received.recv())
        .await
        .expect("a registration arrives")
        .expect("the registry records one");
    assert_eq!(
        defaulted.registrant.user_id,
        Some("default-user".to_owned())
    );
    assert!(defaulted.registrant.api_key.is_none());
    assert_eq!(defaulted.registrant.team_id, None);
    assert_eq!(defaulted.request_tags, None);
    let (second_port, mut second_received) = registry_port(Ok(()));
    let creation = Creation::new(200, batch_body(BATCH_ID), COLLECTION_ROUTE)
        .with_metadata(r#"{"user_api_key_user_id":null}"#);
    drop(run(&handler(None, second_port), &creation));
    let nulled = timeout(Duration::from_secs(5), second_received.recv())
        .await
        .expect("a registration arrives")
        .expect("the registry records one");
    assert_eq!(nulled.registrant.user_id, None);
}

#[tokio::test]
async fn batch_id_scoped_route_skips_registration() {
    let (port, mut received) = registry_port(Ok(()));
    let logging = handler(None, port);
    drop(run(&logging, &Creation::success(MODEL)));
    let other_route = Creation::new(200, batch_body(BATCH_ID), "/v1/messages");
    drop(run(&logging, &other_route));
    yield_now().await;
    assert!(matches!(received.try_recv(), Err(TryRecvError::Empty)));
}

#[tokio::test]
async fn batch_collection_route_variants_register() {
    let (port, mut received) = registry_port(Ok(()));
    let logging = handler(None, port);
    let query_route = Creation::new(
        200,
        batch_body(BATCH_ID),
        "/anthropic/v1/messages/batches?beta=true",
    );
    drop(run(&logging, &query_route));
    let trailing = Creation::new(200, batch_body(BATCH_ID), "/v1/messages/batches/");
    drop(run(&logging, &trailing));
    let first = timeout(Duration::from_secs(5), received.recv())
        .await
        .expect("a registration arrives")
        .expect("the registry records one");
    let second = timeout(Duration::from_secs(5), received.recv())
        .await
        .expect("a second registration arrives")
        .expect("the registry records one");
    assert_eq!(first.batch_object.id, BATCH_ID);
    assert_eq!(second.batch_object.id, BATCH_ID);
}

#[test]
fn batch_missing_registry_warns() {
    let (capture, _lines, _guard) = capture_logs("");
    let creation = Creation::new(200, batch_body(BATCH_ID), COLLECTION_ROUTE)
        .with_request_body(r#"{"model":"claude-x"}"#);
    let (logged, _) = run(&handler(None, None), &creation);
    assert_eq!(
        text(provider_fields(&logged), "batch_job_state"),
        "in_progress"
    );
    assert!(
        capture.contents().contains(
            "Managed files hook not available, cannot store batch object for cost tracking"
        )
    );
}

#[tokio::test]
async fn batch_registry_failure_logs_done_callback() {
    let (port, mut received) = registry_port(Err(RegistryError {
        message: "boom".to_owned(),
    }));
    let (capture, mut lines, _guard) = capture_logs("boom");
    let creation = Creation::new(200, batch_body(BATCH_ID), COLLECTION_ROUTE)
        .with_request_body(r#"{"model":"claude-x"}"#);
    let (logged, _) = run(&handler(None, port), &creation);
    assert_eq!(logged.kwargs.batch_job_state, BatchJobState::InProgress);
    let _registration = timeout(Duration::from_secs(5), received.recv())
        .await
        .expect("a registration arrives")
        .expect("the registry records one");
    timeout(Duration::from_secs(5), lines.recv())
        .await
        .expect("the done callback logs")
        .expect("a log line arrives");
    let logs = capture.contents();
    assert!(
        logs.contains("Failed to store Anthropic batch managed object with unified_object_id=")
    );
    assert!(logs.contains("its cost will not be tracked"));
    assert!(logs.contains("boom"));
}

#[tokio::test]
async fn batch_registry_success_logs_done_callback() {
    let (port, mut received) = registry_port(Ok(()));
    let (capture, mut lines, _guard) = capture_logs("Stored Anthropic batch managed object");
    let creation = Creation::new(200, batch_body(BATCH_ID), COLLECTION_ROUTE)
        .with_request_body(r#"{"model":"claude-x"}"#);
    drop(run(&handler(None, port), &creation));
    let _registration = timeout(Duration::from_secs(5), received.recv())
        .await
        .expect("a registration arrives")
        .expect("the registry records one");
    timeout(Duration::from_secs(5), lines.recv())
        .await
        .expect("the done callback logs")
        .expect("a log line arrives");
    assert!(
        capture
            .contents()
            .contains("Stored Anthropic batch managed object with unified_object_id=")
    );
}

#[test]
fn batch_store_without_runtime_logs_error() {
    let (capture, _lines, _guard) = capture_logs("");
    let (port, _received) = registry_port(Ok(()));
    let logging = handler(None, port);
    let request_metadata = metadata("{}");
    logging.store_batch_managed_object(
        &unified_object_id("anthropic/claude-x", BATCH_ID),
        batch_object(),
        BATCH_ID,
        &request_metadata,
    );
    let logs = capture.contents();
    assert!(logs.contains("Error storing Anthropic batch managed object:"));
    assert!(logs.contains("cannot be spawned without a Tokio runtime"));
}

#[test]
fn batch_unified_object_id_matches_python_base64() {
    assert_eq!(
        unified_object_id("anthropic/claude-sonnet-4-5", "msgbatch_1"),
        SONNET_UNIFIED
    );
    assert_eq!(
        unified_object_id("anthropic/unknown", "msgbatch_1"),
        "bGl0ZWxsbV9wcm94eTttb2RlbF9pZDphbnRocm9waWMvdW5rbm93bjtsbG1fYmF0Y2hfaWQ6bXNnYmF0Y2hfMQ"
    );
}

#[test]
fn batch_unified_object_id_removes_padding() {
    let encoded = unified_object_id("anthropic/m", "bp");
    assert_eq!(
        encoded,
        "bGl0ZWxsbV9wcm94eTttb2RlbF9pZDphbnRocm9waWMvbTtsbG1fYmF0Y2hfaWQ6YnA"
    );
    assert!(!encoded.contains('='));
    let decoded = URL_SAFE_NO_PAD.decode(&encoded).expect("valid base64");
    assert_eq!(
        String::from_utf8(decoded).expect("UTF-8"),
        "litellm_proxy;model_id:anthropic/m;llm_batch_id:bp"
    );
}

#[test]
fn batch_unified_object_id_uses_url_safe_alphabet() {
    let encoded = unified_object_id("\u{00ff}\u{00ff}", "b");
    assert_eq!(
        encoded,
        "bGl0ZWxsbV9wcm94eTttb2RlbF9pZDrDv8O_O2xsbV9iYXRjaF9pZDpi"
    );
    assert!(!encoded.contains('+'));
    assert!(!encoded.contains('/'));
    assert!(!encoded.contains('='));
}

#[test]
fn batch_job_state_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&BatchJobState::InProgress).expect("serialized"),
        "\"in_progress\""
    );
    assert_eq!(
        serde_json::to_string(&BatchJobState::Failed).expect("serialized"),
        "\"failed\""
    );
}

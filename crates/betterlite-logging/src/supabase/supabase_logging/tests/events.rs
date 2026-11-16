use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;
use uuid::Uuid;

use super::{BASE, Captured, MESSAGES, RESPONSE, Recorded, json, logger_on, text};
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::logging_pre_post::hooks::{InputEventSink as _, PreCallEvent};
use crate::supabase::supabase_logging::DEFAULT_TABLE_NAME;
use crate::supabase::supabase_logging::callback::{Outcome, SupabaseCallback};

const TEMPLATE: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": CALL_ID,
    "call_type": "completion",
    "stream": false,
    "response_cost": 0.0000125,
    "response_cost_failure_debug_info": null,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {"user_api_key_user_id": "user-1"},
    "end_user": "proxy-user-1",
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {"user": "end-user-1"},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": RESPONSE,
    "error_str": ERROR_STR
}"#;

fn payload_text(response: &str, error: Option<&str>, call_id: Option<&str>) -> String {
    let error_json = error.map_or_else(
        || "null".to_owned(),
        |message| serde_json::to_string(message).expect("the error encodes as a JSON string"),
    );
    TEMPLATE
        .replace("CALL_ID", call_id.unwrap_or("null"))
        .replace("RESPONSE", response)
        .replace("ERROR_STR", &error_json)
}

fn success_event() -> LoggerEvent {
    event_from(&payload_text(RESPONSE, None, Some(r#""call-1""#)))
}

fn failure_event(error: &str) -> LoggerEvent {
    event_from(&payload_text("null", Some(error), Some(r#""call-1""#)))
}

fn event_from(payload: &str) -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(serde_json::from_str(payload).expect("the fixture payload decodes")),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}

fn callback_fixture(status: u16) -> (SupabaseCallback<Captured>, Captured, TaskTracker) {
    let capture = Captured::answering(status);
    let tracker = TaskTracker::new();
    (
        SupabaseCallback {
            logger: Arc::new(logger_on(BASE, capture.clone(), DEFAULT_TABLE_NAME)),
            tasks: tracker.clone(),
        },
        capture,
        tracker,
    )
}

#[tokio::test]
async fn callback_logger_success_writes_the_row() {
    let (callback, captured, tracker) = callback_fixture(201);
    callback
        .log_success_event(&success_event())
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let recorded = captured.only_request();
    assert_eq!(text(recorded.field("status")), "success");
    assert_eq!(
        text(recorded.field("model")),
        "gpt-4o-2024-08-06",
        "the row takes the model the provider answered with"
    );
    assert_eq!(text(recorded.field("response")), "hi");
    assert_eq!(text(recorded.field("litellm_call_id")), "call-1");
}

#[tokio::test]
async fn callback_logger_failure_writes_the_row() {
    let (callback, captured, tracker) = callback_fixture(201);
    callback
        .log_failure_event(&failure_event("upstream returned 500"))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let recorded = captured.only_request();
    assert_eq!(text(recorded.field("status")), "failure");
    assert_eq!(text(recorded.field("error")), "upstream returned 500");
    assert_eq!(text(recorded.field("model")), "gpt-4o");
    assert_eq!(text(recorded.field("litellm_call_id")), "call-1");
}

#[tokio::test]
async fn callback_logger_async_success_writes_the_row() {
    let (callback, captured, _tracker) = callback_fixture(201);
    callback
        .async_log_success_event(Arc::new(success_event()))
        .await
        .expect("the event is logged");
    assert_eq!(text(captured.only_request().field("status")), "success");
}

#[tokio::test]
async fn callback_logger_async_failure_writes_the_row() {
    let (callback, captured, _tracker) = callback_fixture(201);
    callback
        .async_log_failure_event(Arc::new(failure_event("upstream returned 500")))
        .await
        .expect("the event is logged");
    assert_eq!(text(captured.only_request().field("status")), "failure");
}

#[tokio::test]
async fn callback_logger_success_without_a_call_id_generates_one() {
    let (callback, captured, tracker) = callback_fixture(201);
    callback
        .log_success_event(&event_from(&payload_text(RESPONSE, None, None)))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let recorded = captured.only_request();
    assert!(recorded.url.contains("on_conflict=litellm_call_id"));
    Uuid::parse_str(text(recorded.field("litellm_call_id"))).expect("a fresh uuid4 is written");
}

async fn end_user_written(outcome: Outcome, payload: &str) -> Json {
    let (callback, captured, tracker) = callback_fixture(201);
    let event = event_from(payload);
    let accepted = match outcome {
        Outcome::Success => callback.log_success_event(&event),
        Outcome::Failure => callback.log_failure_event(&event),
    };
    accepted.expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    captured.only_request().field("end_user").clone()
}

fn success_payload() -> String {
    payload_text(RESPONSE, None, Some(r#""call-1""#))
}

fn failure_payload() -> String {
    payload_text("null", Some("upstream returned 500"), Some(r#""call-1""#))
}

const WITH_USER: &str = r#""model_parameters": {"user": "end-user-1"}"#;
const WITHOUT_USER: &str = r#""model_parameters": {}"#;
const COST_OK: &str = r#""response_cost_failure_debug_info": null"#;
const COST_FAILED: &str = r#""response_cost_failure_debug_info": {"error_str": "no price entry"}"#;

#[tokio::test]
async fn callback_logger_success_takes_the_end_user_from_the_user_parameter() {
    let payload = success_payload().replace(r#""end_user": "proxy-user-1""#, r#""end_user": null"#);
    assert_eq!(
        end_user_written(Outcome::Success, &payload).await,
        Json::Text("end-user-1".to_owned())
    );
}

#[tokio::test]
async fn callback_logger_success_ignores_the_proxy_end_user() {
    assert_eq!(
        end_user_written(Outcome::Success, &success_payload()).await,
        Json::Text("end-user-1".to_owned())
    );
    let without_user = success_payload().replace(WITH_USER, WITHOUT_USER);
    assert_eq!(
        end_user_written(Outcome::Success, &without_user).await,
        Json::Text("default".to_owned()),
        "the proxy end-user id is not the request's user parameter"
    );
}

#[tokio::test]
async fn callback_logger_success_defaults_a_user_parameter_that_is_not_text() {
    let payload = success_payload().replace(WITH_USER, r#""model_parameters": {"user": 7}"#);
    assert_eq!(
        end_user_written(Outcome::Success, &payload).await,
        Json::Text("default".to_owned())
    );
}

#[tokio::test]
async fn callback_logger_success_defaults_a_null_user_parameter() {
    let payload = success_payload().replace(WITH_USER, r#""model_parameters": {"user": null}"#);
    assert_eq!(
        end_user_written(Outcome::Success, &payload).await,
        Json::Text("default".to_owned())
    );
}

#[tokio::test]
async fn callback_logger_failure_takes_the_end_user_from_the_user_parameter() {
    assert_eq!(
        end_user_written(Outcome::Failure, &failure_payload()).await,
        Json::Text("end-user-1".to_owned())
    );
}

#[tokio::test]
async fn callback_logger_failure_without_a_user_parameter_writes_a_null_end_user() {
    let payload = failure_payload().replace(WITH_USER, WITHOUT_USER);
    assert_eq!(
        end_user_written(Outcome::Failure, &payload).await,
        Json::Null
    );
}

#[tokio::test]
async fn callback_logger_failure_writes_a_null_user_parameter_as_null() {
    let payload = failure_payload().replace(WITH_USER, r#""model_parameters": {"user": null}"#);
    assert_eq!(
        end_user_written(Outcome::Failure, &payload).await,
        Json::Null
    );
}

#[tokio::test]
async fn callback_logger_success_with_a_cost_failure_posts_nothing() {
    let (callback, captured, tracker) = callback_fixture(201);
    let payload = success_payload().replace(COST_OK, COST_FAILED);
    callback
        .log_success_event(&event_from(&payload))
        .expect("the event is accepted");
    callback
        .async_log_success_event(Arc::new(event_from(&payload)))
        .await
        .expect("the event is logged");
    tracker.close();
    tracker.wait().await;
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn callback_logger_failure_with_a_cost_failure_still_writes_the_row() {
    let payload = failure_payload().replace(COST_OK, COST_FAILED);
    assert_eq!(
        end_user_written(Outcome::Failure, &payload).await,
        Json::Text("end-user-1".to_owned())
    );
}

#[tokio::test]
async fn callback_logger_failure_without_a_call_id_posts_nothing() {
    let (callback, captured, tracker) = callback_fixture(201);
    callback
        .log_failure_event(&event_from(&payload_text(
            "null",
            Some("upstream returned 500"),
            None,
        )))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn callback_logger_success_without_a_response_posts_nothing() {
    let (callback, captured, tracker) = callback_fixture(201);
    callback
        .log_success_event(&event_from(&payload_text(
            "null",
            None,
            Some(r#""call-1""#),
        )))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn callback_logger_failure_without_an_error_posts_nothing() {
    let (callback, captured, tracker) = callback_fixture(201);
    callback
        .log_failure_event(&event_from(&payload_text(
            "null",
            None,
            Some(r#""call-1""#),
        )))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn callback_logger_swallows_a_rejected_row() {
    let (callback, captured, tracker) = callback_fixture(500);
    callback
        .log_success_event(&success_event())
        .expect("the rejected row does not fail the callback");
    tracker.close();
    tracker.wait().await;
    assert_eq!(captured.requests().len(), 1);
    callback
        .async_log_success_event(Arc::new(success_event()))
        .await
        .expect("the rejected row does not fail the async callback");
}

#[tokio::test]
async fn callback_logger_stream_event_posts_nothing() {
    let (callback, captured, tracker) = callback_fixture(201);
    callback
        .log_stream_event(&success_event())
        .expect("the chunk is accepted");
    callback
        .async_log_stream_event(Arc::new(success_event()))
        .await
        .expect("the chunk is logged");
    tracker.close();
    tracker.wait().await;
    assert!(captured.requests().is_empty());
}

#[test]
fn callback_logger_name_is_supabase() {
    let (callback, _captured, _tracker) = callback_fixture(201);
    assert_eq!(callback.callback_name(), "supabase");
}

#[tokio::test]
async fn callback_logger_input_event_writes_the_initiated_row() {
    let (callback, captured, tracker) = callback_fixture(201);
    let messages = json(MESSAGES);
    let details = json(r#"{"user": "end-user-1", "litellm_call_id": "call-1"}"#);
    callback
        .log_input_event(&PreCallEvent {
            model: "gpt-4o",
            messages: &messages,
            details: &details,
        })
        .expect("the pre-call event is accepted");
    tracker.close();
    tracker.wait().await;
    let recorded = captured.only_request();
    assert_eq!(
        recorded.url,
        "https://project.supabase.test/rest/v1/request_logs"
    );
    assert_eq!(
        recorded.body,
        json(
            r#"{
                "model": "gpt-4o",
                "messages": [{"role": "user", "content": "Hello"}],
                "end_user": "end-user-1",
                "status": "initiated",
                "litellm_call_id": "call-1"
            }"#
        )
    );
}

async fn initiated_request(details: &str) -> Recorded {
    let (callback, captured, tracker) = callback_fixture(201);
    let messages = json(MESSAGES);
    let details_json = json(details);
    callback
        .log_input_event(&PreCallEvent {
            model: "gpt-4o",
            messages: &messages,
            details: &details_json,
        })
        .expect("the pre-call event is accepted");
    tracker.close();
    tracker.wait().await;
    captured.only_request()
}

#[tokio::test]
async fn callback_logger_input_event_writes_a_null_user_as_null() {
    let recorded = initiated_request(r#"{"user": null, "litellm_call_id": "call-1"}"#).await;
    assert_eq!(recorded.field("end_user"), &Json::Null);
}

#[tokio::test]
async fn callback_logger_input_event_defaults_a_missing_user() {
    let recorded = initiated_request(r#"{"litellm_call_id": "call-1"}"#).await;
    assert_eq!(
        recorded.field("end_user"),
        &Json::Text("default".to_owned())
    );
}

#[tokio::test]
async fn callback_logger_input_event_with_non_object_details_posts_nothing() {
    let (callback, captured, tracker) = callback_fixture(201);
    let messages = json(MESSAGES);
    let details = json("null");
    callback
        .log_input_event(&PreCallEvent {
            model: "gpt-4o",
            messages: &messages,
            details: &details,
        })
        .expect("the pre-call event is accepted");
    tracker.close();
    tracker.wait().await;
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn callback_logger_input_event_swallows_a_rejected_insert() {
    let (callback, captured, tracker) = callback_fixture(500);
    let messages = json(MESSAGES);
    let details = json(r#"{"user": "end-user-1", "litellm_call_id": "call-1"}"#);
    callback
        .log_input_event(&PreCallEvent {
            model: "gpt-4o",
            messages: &messages,
            details: &details,
        })
        .expect("the rejected insert does not fail the callback");
    tracker.close();
    tracker.wait().await;
    assert_eq!(captured.requests().len(), 1);
}

#[tokio::test]
async fn callback_logger_input_event_without_a_call_id_posts_nothing() {
    let (callback, captured, tracker) = callback_fixture(201);
    let messages = json(MESSAGES);
    let details = json(r#"{"user": "end-user-1"}"#);
    callback
        .log_input_event(&PreCallEvent {
            model: "gpt-4o",
            messages: &messages,
            details: &details,
        })
        .expect("the pre-call event is accepted");
    tracker.close();
    tracker.wait().await;
    assert!(captured.requests().is_empty());
}

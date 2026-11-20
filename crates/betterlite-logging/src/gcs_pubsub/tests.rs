#![cfg(test)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};
use tokio::task::yield_now;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{CALLBACK_NAME, Overrides, PubSubError};
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::gcs_mock::auth::VertexAuthProvider;
use crate::registry::catalog::gcs_pubsub::ProjectionError;
use crate::registry::catalog::gcs_pubsub::flush::spawn_flush_loop;
use crate::registry::catalog::gcs_pubsub::payload::spend_log_payload;

use support::{
    CREDENTIALS, PAYLOAD, PROJECT, TOKEN, TOPIC, capture_logs, configured, event, event_from,
    event_with_metadata, messages, open, open_with, open_with_flush, published, try_open,
};

fn overrides(project: Option<&str>, topic: Option<&str>) -> Overrides {
    Overrides {
        project_id: project.map(str::to_owned),
        topic_id: topic.map(str::to_owned),
        credentials: None,
        use_v1: None,
    }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn json(fixture: &str) -> Json {
    serde_json::from_str(fixture).expect("the expected JSON decodes")
}

fn member<'value>(value: &'value Json, name: &str) -> &'value Json {
    let Json::Object(fields) = value else {
        panic!("the value is an object");
    };
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the object carries {name}"))
}

fn publish_url() -> String {
    format!("https://pubsub.googleapis.com/v1/projects/{PROJECT}/topics/{TOPIC}:publish")
}

#[test]
fn a_missing_project_id_is_an_error() {
    let error = try_open(&[("GCS_PUBSUB_TOPIC_ID", TOPIC)], Overrides::default())
        .err()
        .expect("the project is required");
    assert!(matches!(error, PubSubError::MissingProjectId));
}

#[test]
fn a_missing_topic_id_is_an_error() {
    let error = try_open(&[("GCS_PUBSUB_PROJECT_ID", PROJECT)], Overrides::default())
        .err()
        .expect("the topic is required");
    assert!(matches!(error, PubSubError::MissingTopicId));
}

#[test]
fn an_empty_environment_value_reads_as_unset() {
    let error = try_open(
        &[("GCS_PUBSUB_PROJECT_ID", "")],
        overrides(None, Some(TOPIC)),
    )
    .err()
    .expect("the empty project reads as unset");
    assert!(matches!(error, PubSubError::MissingProjectId));
    let topic_error = try_open(
        &[("GCS_PUBSUB_TOPIC_ID", "")],
        overrides(Some(PROJECT), None),
    )
    .err()
    .expect("the empty topic reads as unset");
    assert!(matches!(topic_error, PubSubError::MissingTopicId));
}

#[test]
fn an_invalid_use_v1_flag_is_reported() {
    let environment = [
        ("GCS_PUBSUB_PROJECT_ID", PROJECT),
        ("GCS_PUBSUB_TOPIC_ID", TOPIC),
        ("GCS_PUB_SUB_USE_V1", "maybe"),
    ];
    let error = try_open(&environment, Overrides::default())
        .err()
        .expect("the flag is not a boolean");
    assert!(matches!(
        error,
        PubSubError::Variable { name, .. } if name == "GCS_PUB_SUB_USE_V1"
    ));
}

#[test]
fn credentials_come_from_the_environment_or_the_constructor() {
    let environment = [
        ("GCS_PUBSUB_PROJECT_ID", PROJECT),
        ("GCS_PUBSUB_TOPIC_ID", TOPIC),
        ("GCS_PATH_SERVICE_ACCOUNT", CREDENTIALS),
    ];
    let from_environment = try_open(&environment, Overrides::default()).expect("the logger builds");
    assert_eq!(
        from_environment
            .settings
            .credentials
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(CREDENTIALS)
    );
    let constructor = Overrides {
        project_id: None,
        topic_id: None,
        credentials: Some(SecretString::from("/keys/override.json")),
        use_v1: None,
    };
    let from_constructor = try_open(&environment, constructor).expect("the logger builds");
    assert_eq!(
        from_constructor
            .settings
            .credentials
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("/keys/override.json")
    );
}

#[tokio::test]
async fn an_empty_credentials_value_reads_as_unset() {
    let environment = [
        ("GCS_PUBSUB_PROJECT_ID", PROJECT),
        ("GCS_PUBSUB_TOPIC_ID", TOPIC),
        ("GCS_PATH_SERVICE_ACCOUNT", ""),
    ];
    let fixture = open(&environment, Overrides::default(), 512);
    fixture.logger.publish_message(&Json::Null).await;
    let access = fixture.auth.access_requests();
    assert_eq!(access.len(), 1);
    assert_eq!(
        access[0]
            .credentials
            .as_ref()
            .map(ExposeSecret::expose_secret),
        None
    );
}

#[tokio::test]
async fn environment_variables_supply_the_project_and_topic() {
    let fixture = open(&configured(), Overrides::default(), 512);
    fixture
        .logger
        .publish_message(&Json::Text("hi".to_owned()))
        .await;
    let requests = fixture.delivery.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, publish_url());
}

#[tokio::test]
async fn constructor_values_win_over_the_environment() {
    let fixture = open(
        &configured(),
        overrides(Some("override-project"), Some("override-topic")),
        512,
    );
    fixture
        .logger
        .publish_message(&Json::Text("hi".to_owned()))
        .await;
    let requests = fixture.delivery.requests();
    assert_eq!(
        requests[0].url,
        "https://pubsub.googleapis.com/v1/projects/override-project/topics/override-topic:publish"
    );
}

#[tokio::test]
async fn an_empty_constructor_value_falls_back_to_the_environment() {
    let fixture = open(&configured(), overrides(Some(""), Some("")), 512);
    fixture
        .logger
        .publish_message(&Json::Text("hi".to_owned()))
        .await;
    let requests = fixture.delivery.requests();
    assert_eq!(requests[0].url, publish_url());
}

#[tokio::test]
async fn request_headers_come_from_the_vertex_token_path() {
    let environment = [
        ("GCS_PUBSUB_PROJECT_ID", PROJECT),
        ("GCS_PUBSUB_TOPIC_ID", TOPIC),
        ("GCS_PATH_SERVICE_ACCOUNT", CREDENTIALS),
    ];
    let fixture = open(&environment, Overrides::default(), 512);
    fixture.logger.publish_message(&Json::Null).await;
    let access = fixture.auth.access_requests();
    assert_eq!(access.len(), 1);
    assert_eq!(access[0].project_id.as_deref(), Some(PROJECT));
    assert_eq!(access[0].custom_llm_provider, VertexAuthProvider::VertexAi);
    assert_eq!(
        access[0]
            .credentials
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(CREDENTIALS)
    );
    let tokens = fixture.auth.token_requests();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].model, "pub-sub");
    assert_eq!(tokens[0].vertex_project.as_deref(), Some(PROJECT));
    assert_eq!(tokens[0].custom_llm_provider, VertexAuthProvider::VertexAi);
    assert_eq!(
        tokens[0]
            .auth_header
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(TOKEN)
    );
    assert_eq!(
        tokens[0]
            .vertex_credentials
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(CREDENTIALS)
    );
}

#[tokio::test]
async fn the_request_carries_a_bearer_authorization_header() {
    let fixture = open(&configured(), Overrides::default(), 512);
    fixture.logger.publish_message(&Json::Null).await;
    let requests = fixture.delivery.requests();
    let headers = &requests[0].headers;
    let authorization = format!("Bearer {TOKEN}");
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some(authorization.as_str())
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
}

#[tokio::test]
async fn the_publish_body_carries_one_base64_message() {
    let fixture = open(&configured(), Overrides::default(), 512);
    fixture
        .logger
        .publish_message(&Json::Text("hi".to_owned()))
        .await;
    let requests = fixture.delivery.requests();
    let Json::Object(body) = &requests[0].body else {
        panic!("the publish body is an object");
    };
    let Some(Json::List(entries)) = body.get("messages") else {
        panic!("the publish body carries messages");
    };
    assert_eq!(entries.len(), 1);
    let Json::Object(entry) = &entries[0] else {
        panic!("the message entry is an object");
    };
    assert_eq!(entry.len(), 1);
    let Some(Json::Text(data)) = entry.get("data") else {
        panic!("the message entry carries data");
    };
    assert_eq!(data, "aGk=");
}

#[tokio::test]
async fn a_string_message_is_published_unchanged() {
    let fixture = open(&configured(), Overrides::default(), 512);
    fixture
        .logger
        .publish_message(&Json::Text("plain text".to_owned()))
        .await;
    let requests = fixture.delivery.requests();
    assert_eq!(messages(&requests[0]), vec!["plain text".to_owned()]);
}

#[tokio::test]
async fn a_json_message_is_published_as_compact_json() {
    let fixture = open(&configured(), Overrides::default(), 512);
    fixture
        .logger
        .publish_message(&json(r#"{"model": "gpt-4o", "n": 1}"#))
        .await;
    let requests = fixture.delivery.requests();
    assert_eq!(
        messages(&requests[0]),
        vec![r#"{"model":"gpt-4o","n":1}"#.to_owned()]
    );
}

#[tokio::test]
async fn a_success_event_is_flushed_at_the_batch_size() {
    let fixture = open(&configured(), Overrides::default(), 2);
    fixture
        .logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    assert!(fixture.delivery.requests().is_empty());
    assert_eq!(fixture.logger.queue.pending().len(), 1);
    fixture
        .logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    assert_eq!(fixture.delivery.requests().len(), 2);
    assert!(fixture.logger.queue.pending().is_empty());
}

#[tokio::test]
async fn send_batch_publishes_each_queued_message_separately() {
    let fixture = open(&configured(), Overrides::default(), 512);
    let first = event_from(&PAYLOAD.replace("chatcmpl-1", "chatcmpl-2"));
    let second = event_from(&PAYLOAD.replace("chatcmpl-1", "chatcmpl-3"));
    let third = event_from(&PAYLOAD.replace("chatcmpl-1", "chatcmpl-4"));
    fixture
        .logger
        .async_log_success_event(Arc::new(first))
        .await
        .expect("the event is accepted");
    fixture
        .logger
        .async_log_success_event(Arc::new(second))
        .await
        .expect("the event is accepted");
    fixture
        .logger
        .async_log_success_event(Arc::new(third))
        .await
        .expect("the event is accepted");
    assert!(fixture.delivery.requests().is_empty());
    fixture.logger.send_batch().await;
    let requests = fixture.delivery.requests();
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|recorded| messages(recorded).len() == 1)
    );
    let bodies: Vec<Json> = requests.iter().map(published).collect();
    assert_eq!(member(&bodies[0], "id"), &text("chatcmpl-2"));
    assert_eq!(member(&bodies[1], "id"), &text("chatcmpl-3"));
    assert_eq!(member(&bodies[2], "id"), &text("chatcmpl-4"));
}

#[tokio::test]
async fn an_empty_batch_publishes_nothing() {
    let fixture = open(&configured(), Overrides::default(), 512);
    fixture.logger.send_batch().await;
    assert!(fixture.delivery.requests().is_empty());
}

#[tokio::test]
async fn an_event_enqueued_during_a_flush_stays_queued() {
    let fixture = open(&configured(), Overrides::default(), 512);
    fixture.delivery.hold();
    fixture
        .logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    let delivery = fixture.delivery.clone();
    let logger = fixture.logger;
    let flush = logger.send_batch();
    let enqueue = async {
        yield_now().await;
        logger
            .async_log_success_event(Arc::new(event()))
            .await
            .expect("the event is accepted");
        delivery.release();
    };
    tokio::join!(flush, enqueue);
    assert_eq!(delivery.requests().len(), 1);
    assert_eq!(logger.queue.pending().len(), 1);
}

#[tokio::test]
async fn a_failed_publish_does_not_stay_queued() {
    let fixture = open(&configured(), Overrides::default(), 512);
    fixture
        .logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    fixture.delivery.fail();
    let (_guard, logs) = capture_logs();
    fixture.logger.send_batch().await;
    assert_eq!(fixture.delivery.requests().len(), 1);
    assert!(fixture.logger.queue.pending().is_empty());
    assert!(logs.contents().contains("Pub/Sub publish error"));
    fixture.delivery.recover();
    fixture.logger.send_batch().await;
    assert_eq!(fixture.delivery.requests().len(), 1);
}

#[tokio::test]
async fn an_accepted_status_counts_as_delivered() {
    let fixture = open(&configured(), Overrides::default(), 512);
    fixture
        .logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    fixture.delivery.answer(202);
    let (_guard, logs) = capture_logs();
    fixture.logger.send_batch().await;
    assert!(fixture.logger.queue.pending().is_empty());
    assert!(!logs.contents().contains("Pub/Sub publish error"));
}

#[tokio::test]
async fn another_status_is_logged_as_an_error() {
    let fixture = open(&configured(), Overrides::default(), 512);
    fixture
        .logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    fixture.delivery.answer(500);
    let (_guard, logs) = capture_logs();
    fixture.logger.send_batch().await;
    assert!(logs.contents().contains("status 500"));
    assert!(fixture.logger.queue.pending().is_empty());
}

fn rejected(_event: &LoggerEvent) -> Result<Json, ProjectionError> {
    Err(ProjectionError::Time { field: "startTime" })
}

#[tokio::test]
async fn a_projection_error_is_logged_and_publishes_nothing() {
    let override_v1 = Overrides {
        project_id: None,
        topic_id: None,
        credentials: None,
        use_v1: Some(true),
    };
    let fixture = open_with(&configured(), override_v1, 1, rejected);
    let (_guard, logs) = capture_logs();
    fixture
        .logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the projection error does not propagate");
    assert!(fixture.delivery.requests().is_empty());
    assert!(logs.contents().contains("PubSub Layer Error"));
    assert!(fixture.logger.queue.pending().is_empty());
}

#[tokio::test]
async fn an_unreadable_timestamp_is_reported_by_the_projection() {
    let override_v1 = Overrides {
        project_id: None,
        topic_id: None,
        credentials: None,
        use_v1: Some(true),
    };
    let fixture = open_with(&configured(), override_v1, 1, spend_log_payload);
    let broken = event_from(&PAYLOAD.replace("1704067200.0", "100000000000000000000"));
    let (_guard, logs) = capture_logs();
    fixture
        .logger
        .async_log_success_event(Arc::new(broken))
        .await
        .expect("the projection error does not propagate");
    assert!(fixture.delivery.requests().is_empty());
    assert!(logs.contents().contains("startTime"));
}

#[tokio::test]
async fn callback_logger_handles_success_and_failure_events() {
    let fixture = open(&configured(), Overrides::default(), 1);
    assert_eq!(fixture.logger.callback_name(), CALLBACK_NAME);
    fixture
        .logger
        .async_log_failure_event(Arc::new(event()))
        .await
        .expect("the failure event is accepted");
    assert!(fixture.delivery.requests().is_empty());
    fixture
        .logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the success event is accepted");
    assert_eq!(fixture.delivery.requests().len(), 1);
}

#[tokio::test]
async fn callback_logger_publishes_the_standard_payload_by_default() {
    let fixture = open(&configured(), Overrides::default(), 1);
    fixture
        .logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    let requests = fixture.delivery.requests();
    let body = published(&requests[0]);
    assert_eq!(member(&body, "id"), &text("chatcmpl-1"));
    assert_eq!(member(&body, "trace_id"), &text("trace-1"));
    assert!(matches!(member(&body, "status_fields"), Json::Object(_)));
}

#[tokio::test]
async fn callback_logger_publishes_the_spend_projection_for_the_v1_flag() {
    let fixture = open(&v1_environment(), Overrides::default(), 1);
    let with_metadata = event_with_metadata(&[
        ("user_api_key", "sk-secret"),
        ("billing_agent_id", "agent-9"),
        ("user_api_key_end_user_id", "end-user-2"),
        ("user_api_key_user_id", "user-7"),
    ]);
    let body = projected(&fixture, with_metadata).await;
    assert_eq!(member(&body, "request_id"), &text("chatcmpl-1"));
    assert_eq!(member(&body, "call_type"), &text("completion"));
    assert_eq!(member(&body, "model"), &text("gpt-4o"));
    assert_eq!(
        member(&body, "startTime"),
        &text("2024-01-01T00:00:00+00:00")
    );
    assert_eq!(
        member(&body, "endTime"),
        &text("2024-01-01T00:00:01.500000+00:00")
    );
    assert_eq!(
        member(&body, "completionStartTime"),
        &text("2024-01-01T00:00:00.500000+00:00")
    );
    assert_eq!(member(&body, "session_id"), &text("trace-1"));
    assert_eq!(member(&body, "cache_key"), &text("Cache OFF"));
    assert_eq!(member(&body, "cache_hit"), &text("False"));
    assert_eq!(member(&body, "end_user"), &text("end-user-2"));
    assert_eq!(member(&body, "user"), &text("user-7"));
    assert_eq!(member(&body, "request_tags"), &text("[\"team-a\"]"));
    assert_eq!(member(&body, "billing_agent_id"), &text("agent-9"));
    assert_eq!(member(&body, "messages"), &text("{}"));
    assert_eq!(member(&body, "request_duration_ms"), &json("1500"));
    assert_eq!(member(&body, "total_tokens"), &json("5"));
    assert_eq!(member(&body, "spend"), &json("0.5"));
    assert_eq!(member(&body, "status"), &text("success"));
    let clean = cleaned(&body);
    assert_eq!(member(&clean, "billing_agent_id"), &text("agent-9"));
    assert_eq!(member(&clean, "additional_usage_values"), &json("{}"));
    assert_ne!(member(&clean, "user_api_key"), &text("sk-secret"));
    assert_eq!(member(&body, "api_key"), member(&clean, "user_api_key"));
}

fn v1_environment() -> [(&'static str, &'static str); 3] {
    [
        ("GCS_PUBSUB_PROJECT_ID", PROJECT),
        ("GCS_PUBSUB_TOPIC_ID", TOPIC),
        ("GCS_PUB_SUB_USE_V1", "true"),
    ]
}

async fn projected(fixture: &support::Fixture, event: LoggerEvent) -> Json {
    fixture
        .logger
        .async_log_success_event(Arc::new(event))
        .await
        .expect("the event is accepted");
    let requests = fixture.delivery.requests();
    assert_eq!(requests.len(), 1);
    published(&requests[0])
}

fn cleaned(body: &Json) -> Json {
    let Json::Text(metadata) = member(body, "metadata") else {
        panic!("the metadata is a JSON string");
    };
    json(metadata)
}

fn oauth_stamp(sent: bool) -> IndexMap<String, Json> {
    let mut merged = IndexMap::new();
    merged.insert("used_client_oauth_token".to_owned(), Json::Bool(sent));
    merged
}

#[tokio::test]
async fn an_aretrieve_batch_event_marks_its_request_id() {
    let fixture = open(&v1_environment(), Overrides::default(), 1);
    let batch = PAYLOAD.replace(
        "\"call_type\": \"completion\"",
        "\"call_type\": \"aretrieve_batch\"",
    );
    let body = projected(&fixture, event_from(&batch)).await;
    assert_eq!(member(&body, "request_id"), &text("chatcmpl-1_batch_cost"));
    assert_eq!(member(&body, "session_id"), &text("chatcmpl-1"));
}

#[tokio::test]
async fn a_cache_hit_event_gets_a_unique_request_id() {
    let fixture = open(&v1_environment(), Overrides::default(), 1);
    let hit = PAYLOAD.replace("\"cache_hit\": false", "\"cache_hit\": true");
    let body = projected(&fixture, event_from(&hit)).await;
    assert_eq!(
        member(&body, "request_id"),
        &text("chatcmpl-1_cache_hit1704067201.5")
    );
    assert_eq!(member(&body, "cache_hit"), &text("True"));
}

#[tokio::test]
async fn a_missing_request_id_falls_back_to_the_missing_marker() {
    let fixture = open(&v1_environment(), Overrides::default(), 1);
    let anonymous = PAYLOAD
        .replace("\"id\": \"chatcmpl-1\"", "\"id\": \"\"")
        .replace(
            "\"litellm_call_id\": \"call-1\"",
            "\"litellm_call_id\": null",
        );
    let body = projected(&fixture, event_from(&anonymous)).await;
    assert_eq!(member(&body, "request_id"), &text("None"));
}

#[tokio::test]
async fn the_client_oauth_token_is_kept_only_for_anthropic() {
    let fixture = open(&v1_environment(), Overrides::default(), 1);
    let mut openai_event = event();
    openai_event.litellm_metadata = Arc::new(oauth_stamp(true));
    let openai_body = projected(&fixture, openai_event).await;
    assert_eq!(
        member(&cleaned(&openai_body), "used_client_oauth_token"),
        &json("false")
    );
    let anthropic_payload = PAYLOAD.replace(
        "\"custom_llm_provider\": \"openai\"",
        "\"custom_llm_provider\": \"anthropic\"",
    );
    let mut anthropic_event = event_from(&anthropic_payload);
    anthropic_event.litellm_metadata = Arc::new(oauth_stamp(true));
    let anthropic_body = projected(&fixture, anthropic_event).await;
    assert_eq!(
        member(&cleaned(&anthropic_body), "used_client_oauth_token"),
        &json("true")
    );
    let mut unstamped = event_from(&anthropic_payload);
    unstamped.litellm_metadata = Arc::new(oauth_stamp(false));
    let unstamped_body = projected(&fixture, unstamped).await;
    assert_eq!(
        member(&cleaned(&unstamped_body), "used_client_oauth_token"),
        &json("false")
    );
}

#[tokio::test]
async fn the_spillover_headers_reach_the_projected_metadata() {
    let fixture = open(&v1_environment(), Overrides::default(), 1);
    let spilled = PAYLOAD.replace(
        "\"hidden_params\": {}",
        "\"hidden_params\": {\"additional_headers\": {\
         \"llm_provider-x-ms-is-spilled-over\": \"true\", \
         \"llm_provider-x-ms-spillover-from-deployment\": \"dep-1\"}}",
    );
    let body = projected(&fixture, event_from(&spilled)).await;
    let clean = cleaned(&body);
    assert_eq!(
        member(member(&clean, "azure_spillover"), "from_deployment"),
        &text("dep-1")
    );
}

#[tokio::test]
async fn the_namespaced_mcp_tool_reaches_the_projected_record() {
    let fixture = open(&v1_environment(), Overrides::default(), 1);
    let tool = PAYLOAD.replace(
        "\"user_api_key_end_user_id\": \"end-user-1\"",
        "\"user_api_key_end_user_id\": \"end-user-1\", \
         \"mcp_tool_call_metadata\": {\"namespaced_tool_name\": \"tool-1\"}",
    );
    let body = projected(&fixture, event_from(&tool)).await;
    assert_eq!(member(&body, "mcp_namespaced_tool_name"), &text("tool-1"));
}

#[tokio::test]
async fn the_guardrail_information_reaches_the_projected_metadata() {
    let fixture = open(&v1_environment(), Overrides::default(), 1);
    let guarded = PAYLOAD.replace(
        "\"error_str\": null",
        "\"error_str\": null, \"guardrail_information\": [{\"guardrail_name\": \"my-guard\"}]",
    );
    let body = projected(&fixture, event_from(&guarded)).await;
    let clean = cleaned(&body);
    let Json::List(entries) = member(&clean, "guardrail_information") else {
        panic!("the guardrail information is a list");
    };
    assert_eq!(member(&entries[0], "guardrail_name"), &text("my-guard"));
}

#[tokio::test]
async fn the_flush_loop_publishes_on_its_interval() {
    let fixture = open_with_flush(
        &configured(),
        Overrides::default(),
        Duration::from_millis(10),
    );
    let logger = Arc::new(fixture.logger);
    let tracker = TaskTracker::new();
    let shutdown = CancellationToken::new();
    spawn_flush_loop(&logger, &tracker, shutdown.clone());
    logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    assert!(fixture.delivery.requests().is_empty());
    sleep(Duration::from_millis(500)).await;
    assert_eq!(fixture.delivery.requests().len(), 1);
    shutdown.cancel();
    tracker.close();
    timeout(Duration::from_secs(2), tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
}

#[tokio::test]
async fn the_constructor_use_v1_flag_wins_over_the_environment() {
    let disabled = [
        ("GCS_PUBSUB_PROJECT_ID", PROJECT),
        ("GCS_PUBSUB_TOPIC_ID", TOPIC),
        ("GCS_PUB_SUB_USE_V1", "false"),
    ];
    let enabled = Overrides {
        project_id: None,
        topic_id: None,
        credentials: None,
        use_v1: Some(true),
    };
    let projection_fixture = open(&disabled, enabled, 1);
    projection_fixture
        .logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    let projection = published(&projection_fixture.delivery.requests()[0]);
    assert_eq!(member(&projection, "request_id"), &text("chatcmpl-1"));
    let switched_on = [
        ("GCS_PUBSUB_PROJECT_ID", PROJECT),
        ("GCS_PUBSUB_TOPIC_ID", TOPIC),
        ("GCS_PUB_SUB_USE_V1", "true"),
    ];
    let standard = Overrides {
        project_id: None,
        topic_id: None,
        credentials: None,
        use_v1: Some(false),
    };
    let standard_fixture = open(&switched_on, standard, 1);
    standard_fixture
        .logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    let body = published(&standard_fixture.delivery.requests()[0]);
    assert_eq!(member(&body, "id"), &text("chatcmpl-1"));
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{build, concrete_logger};
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::datadog_llm_obs::records::DdMessage;
use crate::exporter_options::redaction::REDACTED_TEXT;
use crate::fields::payload::StandardLoggingPayload;
use crate::registry::catalog::{CatalogError, CatalogInputs, LoggerCatalog};

fn inputs(source: &str, environment: BTreeMap<String, String>) -> CatalogInputs {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("valid config");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("http client"),
        env: Arc::new(environment),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn empty() -> CatalogInputs {
    inputs("", BTreeMap::new())
}

#[tokio::test]
async fn callback_logger_builds_in_agent_mode() {
    let environment =
        BTreeMap::from([("LITELLM_DD_AGENT_HOST".to_owned(), "agent.local".to_owned())]);
    let logger = build(&inputs("", environment)).expect("the logger builds in agent mode");
    assert_eq!(logger.callback_name(), "datadog_llm_observability");
}

#[tokio::test]
async fn callback_logger_builds_in_direct_api_mode() {
    let environment = BTreeMap::from([
        ("DD_API_KEY".to_owned(), "secret-key".to_owned()),
        ("DD_SITE".to_owned(), "datadoghq.eu".to_owned()),
    ]);
    let logger = build(&inputs("", environment)).expect("the logger builds with credentials");
    assert_eq!(logger.callback_name(), "datadog_llm_observability");
}

#[tokio::test]
async fn callback_logger_reports_missing_credentials() {
    let Err(error) = build(&empty()) else {
        panic!("a build without credentials must fail");
    };
    assert!(matches!(error, CatalogError::Init { .. }));
    assert!(error.to_string().contains("DD_API_KEY"));
}

#[tokio::test]
async fn callback_logger_reads_turn_off_message_logging() {
    let source =
        "callback_settings:\n  datadog_llm_observability:\n    turn_off_message_logging: true\n";
    let environment =
        BTreeMap::from([("LITELLM_DD_AGENT_HOST".to_owned(), "agent.local".to_owned())]);
    let logger = build(&inputs(source, environment)).expect("the logger builds");
    assert!(logger.turn_off_message_logging());
}

#[tokio::test]
async fn callback_logger_ignores_an_unknown_callback_setting() {
    let source = "callback_settings:\n  datadog_llm_observability:\n    not_a_field: true\n";
    let environment =
        BTreeMap::from([("LITELLM_DD_AGENT_HOST".to_owned(), "agent.local".to_owned())]);
    let logger = build(&inputs(source, environment)).expect("the logger builds");
    assert!(!logger.turn_off_message_logging());
}

#[tokio::test]
async fn callback_logger_is_cached_by_the_catalog() {
    let environment =
        BTreeMap::from([("LITELLM_DD_AGENT_HOST".to_owned(), "agent.local".to_owned())]);
    let catalog = LoggerCatalog::new(inputs("", environment));
    let first = catalog
        .resolve_logger("datadog_llm_observability")
        .expect("the callback name is known")
        .expect("the logger builds");
    let second = catalog
        .resolve_logger("datadog_llm_observability")
        .expect("the callback name is known")
        .expect("the logger is cached");
    assert!(Arc::ptr_eq(&first, &second));
}

const REDACTION_PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
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
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {"tools": [{"type": "function", "function": {"name": "get_weather", "parameters": {"type": "object"}}}]},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "hi"}],
    "response": {"choices": [{"message": {"role": "assistant", "content": "hello"}}]},
    "metadata": {}
}"#;

fn redaction_event() -> Arc<LoggerEvent> {
    let payload: StandardLoggingPayload =
        serde_json::from_str(REDACTION_PAYLOAD).expect("the fixture is a standard logging payload");
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

fn agent_inputs(source: &str) -> CatalogInputs {
    inputs(
        source,
        BTreeMap::from([("LITELLM_DD_AGENT_HOST".to_owned(), "agent.local".to_owned())]),
    )
}

#[tokio::test]
async fn callback_logger_redacts_when_the_global_flag_is_set() {
    let logger = concrete_logger(&agent_inputs(
        "litellm_settings:\n  turn_off_message_logging: true\n",
    ))
    .expect("the logger builds");
    logger
        .async_log_success_event(redaction_event())
        .await
        .expect("the event is accepted");
    let queued = logger.queue.pending();
    assert_eq!(queued.len(), 1);
    let meta = queued[0].meta.clone().expect("the span carries meta");
    assert!(meta.tool_definitions.is_empty());
    let input = meta.input.expect("the input is present");
    let DdMessage::Message(message) = &input.messages[0] else {
        panic!("the redacted message stays a message record");
    };
    assert_eq!(message.content, Json::Text(REDACTED_TEXT.to_owned()));
}

#[tokio::test]
async fn callback_logger_keeps_messages_when_the_global_flag_is_unset() {
    let logger = concrete_logger(&agent_inputs("")).expect("the logger builds");
    logger
        .async_log_success_event(redaction_event())
        .await
        .expect("the event is accepted");
    let queued = logger.queue.pending();
    assert_eq!(queued.len(), 1);
    let meta = queued[0].meta.clone().expect("the span carries meta");
    assert_eq!(meta.tool_definitions.len(), 1);
    let input = meta.input.expect("the input is present");
    let DdMessage::Message(message) = &input.messages[0] else {
        panic!("the input message projects to a message record");
    };
    assert_eq!(message.content, Json::Text("hi".to_owned()));
}

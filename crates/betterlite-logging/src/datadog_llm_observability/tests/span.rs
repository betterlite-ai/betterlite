use std::convert::Infallible;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::callback_logger::CallbackLogger as _;
use crate::datadog_llm_obs::records::DdMessage;
use crate::datadog_llm_obs::span_records::{LLMObsPayload, SpanKind, SpanStatus};
use crate::datadog_llm_obs::values::Object;
use crate::datadog_llm_observability::settings::DatadogLlmObsSettings;
use crate::datadog_llm_observability::span::{
    SpanContext, SpanError, SpanRequest, logging_is_off, span_payload,
};
use crate::datadog_llm_observability::tests::support::{self, TestCounter};
use crate::exporter_options::options::CustomLoggerOptions;
use crate::exporter_options::redaction::REDACTED_TEXT;
use crate::fields::payload::StandardLoggingPayload;

fn options(turn_off_message_logging: bool) -> CustomLoggerOptions {
    CustomLoggerOptions {
        turn_off_message_logging,
        message_logging: true,
    }
}

fn settings(turn_off_message_logging: bool) -> DatadogLlmObsSettings {
    let mut settings = support::resolve(&support::agent_init(), &support::environment(&[]))
        .expect("the settings resolve");
    settings.options = options(turn_off_message_logging);
    settings
}

fn object(fields: &[(&str, Json)]) -> Object {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

async fn assemble(
    settings: &DatadogLlmObsSettings,
    payload: &StandardLoggingPayload,
    metadata: &Object,
    redact: bool,
) -> Result<LLMObsPayload, SpanError<Infallible>> {
    let counter = TestCounter::default();
    let context = SpanContext {
        settings,
        counter: &counter,
        generate_id: &support::generated_id,
    };
    let request = SpanRequest {
        payload,
        request_metadata: metadata,
        redact,
        apm_id: None,
    };
    span_payload(&request, &context).await
}

#[tokio::test]
async fn datadog_llm_obs_span_identity_defaults_apply_without_caller_metadata() {
    let payload = support::standard_payload("chatcmpl-1");
    let span = assemble(&settings(false), &payload, &Object::new(), false)
        .await
        .expect("the span assembles");
    assert_eq!(span.parent_id.as_deref(), Some("undefined"));
    assert_eq!(span.trace_id.as_deref(), Some("trace-1"));
    assert_eq!(span.span_id.as_deref(), Some("generated-id"));
    assert_eq!(span.name.as_deref(), Some("litellm_llm_call"));
    assert_eq!(span.status, Some(SpanStatus::Ok));
    assert_eq!(span.apm_id, None);
    let meta = span.meta.expect("the span carries meta");
    assert_eq!(meta.kind, Some(SpanKind::Llm));
    assert!(meta.error.is_none());
}

#[tokio::test]
async fn datadog_llm_obs_span_identity_follows_the_caller_metadata() {
    let payload = support::standard_payload("chatcmpl-1");
    let metadata = object(&[
        ("span_id", text("caller-span")),
        ("name", text("caller-name")),
        ("parent_id", text("caller-parent")),
    ]);
    let span = assemble(&settings(false), &payload, &metadata, false)
        .await
        .expect("the span assembles");
    assert_eq!(span.span_id.as_deref(), Some("caller-span"));
    assert_eq!(span.name.as_deref(), Some("caller-name"));
    assert_eq!(span.parent_id.as_deref(), Some("caller-parent"));
}

#[tokio::test]
async fn datadog_llm_obs_span_identity_reads_scalars_as_text() {
    let payload = support::standard_payload("chatcmpl-1");
    let metadata = object(&[
        ("span_id", Json::Number(serde_json::Number::from(7_i32))),
        ("parent_id", Json::Bool(true)),
    ]);
    let span = assemble(&settings(false), &payload, &metadata, false)
        .await
        .expect("the span assembles");
    assert_eq!(span.span_id.as_deref(), Some("7"));
    assert_eq!(span.parent_id.as_deref(), Some("true"));
    assert_eq!(span.name.as_deref(), Some("litellm_llm_call"));
}

#[tokio::test]
async fn datadog_llm_obs_an_empty_parent_id_is_undefined() {
    let payload = support::standard_payload("chatcmpl-1");
    let metadata = object(&[("parent_id", text(""))]);
    let span = assemble(&settings(false), &payload, &metadata, false)
        .await
        .expect("the span assembles");
    assert_eq!(span.parent_id.as_deref(), Some("undefined"));
}

#[tokio::test]
async fn datadog_llm_obs_span_timing_is_nanoseconds() {
    let payload = support::standard_payload("chatcmpl-1");
    let span = assemble(&settings(false), &payload, &Object::new(), false)
        .await
        .expect("the span assembles");
    assert_eq!(span.start_ns, Some(1_704_067_200_000_000_000));
    assert_eq!(span.duration, Some(1_500_000_000));
}

#[tokio::test]
async fn datadog_llm_obs_span_timing_overflow_is_reported() {
    let mut payload = support::standard_payload("chatcmpl-1");
    payload.start_time = Decimal::from(1_000_000_000_000_000_000_i64);
    let error = assemble(&settings(false), &payload, &Object::new(), false)
        .await
        .expect_err("the timestamp is too large");
    assert!(matches!(error, SpanError::TimingOverflow));
}

#[tokio::test]
async fn datadog_llm_obs_span_timing_of_an_instant_call_is_zero() {
    let mut payload = support::standard_payload("chatcmpl-1");
    payload.end_time = payload.start_time;
    let span = assemble(&settings(false), &payload, &Object::new(), false)
        .await
        .expect("the span assembles");
    assert_eq!(span.duration, Some(0));
}

#[tokio::test]
async fn datadog_llm_obs_span_status_reflects_the_error_information() {
    let mut payload = support::failure_payload("chatcmpl-1");
    payload.error_information = Some(
        serde_json::from_str(
            r#"{"error_message": "boom", "error_class": "BadRequestError", "traceback": "trace"}"#,
        )
        .expect("the error information parses"),
    );
    let span = assemble(&settings(false), &payload, &Object::new(), false)
        .await
        .expect("the span assembles");
    assert_eq!(span.status, Some(SpanStatus::Error));
    let error = span
        .meta
        .expect("the span carries meta")
        .error
        .expect("the error is attached");
    assert_eq!(error.message.as_deref(), Some("boom"));
    assert_eq!(error.kind.as_deref(), Some("BadRequestError"));
    assert_eq!(error.stack.as_deref(), Some("trace"));
}

#[tokio::test]
async fn datadog_llm_obs_span_status_stays_ok_without_error_information() {
    let payload = support::failure_payload("chatcmpl-1");
    let span = assemble(&settings(false), &payload, &Object::new(), false)
        .await
        .expect("the span assembles");
    assert_eq!(span.status, Some(SpanStatus::Ok));
    assert!(span.meta.expect("the span carries meta").error.is_none());
}

#[tokio::test]
async fn datadog_llm_obs_span_messages_come_from_the_payload() {
    let payload = support::standard_payload("chatcmpl-1");
    let span = assemble(&settings(false), &payload, &Object::new(), false)
        .await
        .expect("the span assembles");
    let meta = span.meta.expect("the span carries meta");
    let input = meta.input.expect("the input is present");
    let DdMessage::Message(asked) = &input.messages[0] else {
        panic!("the input message projects to a message record");
    };
    assert_eq!(asked.role, "user");
    assert_eq!(asked.content, text("hi"));
    let output = meta.output.expect("the output is present");
    let DdMessage::Message(answered) = &output.messages[0] else {
        panic!("the output message projects to a message record");
    };
    assert_eq!(answered.role, "assistant");
    assert_eq!(answered.content, text("hello"));
}

#[tokio::test]
async fn datadog_llm_obs_redaction_replaces_messages_and_drops_tool_definitions() {
    let mut payload = support::standard_payload("chatcmpl-1");
    payload.model_parameters.insert(
        "tools".to_owned(),
        serde_json::from_str(
            r#"[{"type": "function", "function": {"name": "get_weather", "parameters": {"type": "object"}}}]"#,
        )
        .expect("the tools parse"),
    );
    let plain = assemble(&settings(false), &payload, &Object::new(), false)
        .await
        .expect("the span assembles");
    assert_eq!(
        plain
            .meta
            .expect("the span carries meta")
            .tool_definitions
            .len(),
        1
    );
    let redacted = assemble(&settings(true), &payload, &Object::new(), true)
        .await
        .expect("the span assembles");
    let meta = redacted.meta.expect("the span carries meta");
    assert!(meta.tool_definitions.is_empty());
    let input = meta.input.expect("the input is present");
    let DdMessage::Message(message) = &input.messages[0] else {
        panic!("the redacted message stays a message record");
    };
    assert_eq!(message.role, "user");
    assert_eq!(message.content, text(REDACTED_TEXT));
}

#[tokio::test]
async fn datadog_llm_obs_span_tags_carry_request_team_and_cost_dimensions() {
    let payload = tagged_payload();
    let span = assemble(&settings(false), &payload, &Object::new(), false)
        .await
        .expect("the span assembles");
    assert_eq!(span.tags, Some(expected_tags()));
}

#[tokio::test]
async fn datadog_llm_obs_span_metadata_declares_the_cost_dimensions() {
    let payload = tagged_payload();
    let span = assemble(&settings(false), &payload, &Object::new(), false)
        .await
        .expect("the span assembles");
    let metadata = span
        .meta
        .expect("the span carries meta")
        .metadata
        .expect("the metadata is present");
    let expected = Json::List(
        [
            "team",
            "user",
            "model_group",
            "router_tier",
            "router_cause",
            "router_escalated",
            "routed_model",
        ]
        .iter()
        .map(|name| text(name))
        .collect(),
    );
    let rendered = metadata.get("_dd").expect("the cost tags are declared");
    assert_eq!(support::field(rendered, &["cost_tags"]), Some(&expected));
}

fn tagged_payload() -> StandardLoggingPayload {
    let mut payload = support::standard_payload("chatcmpl-1");
    payload.request_tags = vec![text("prod"), Json::Null];
    payload.model_group = Some("chat-models".to_owned());
    payload.metadata.user_api_key_user_id = Some("alice".to_owned());
    payload.metadata.team_alias = Some("Engineering Team".to_owned());
    payload.metadata.routing_decision = Some(
        serde_json::from_str(
            r#"{"tier": "low", "cause": "heuristic", "escalated": true, "routed_model": "gpt-4o-mini", "signals": ["a long prompt"]}"#,
        )
        .expect("the routing decision parses"),
    );
    payload
}

fn expected_tags() -> Vec<String> {
    let tags = [
        "env:unknown",
        "service:litellm-server",
        "version:unknown",
        "HOSTNAME:",
        "POD_NAME:unknown",
        "request_tag:prod",
        "request_tag:none",
        "team:engineering_team",
        "user:alice",
        "model_group:chat-models",
        "router_tier:low",
        "router_cause:heuristic",
        "router_escalated:true",
        "routed_model:gpt-4o-mini",
    ];
    tags.iter().map(|tag| (*tag).to_owned()).collect()
}

#[test]
fn datadog_llm_obs_logging_is_off_follows_the_flags_and_the_request() {
    let open = options(false);
    let mut request = Object::new();
    assert!(!logging_is_off(&open, &request, false));
    request.insert("turn_off_message_logging".to_owned(), Json::Bool(true));
    assert!(logging_is_off(&open, &request, false));
    request.insert("turn_off_message_logging".to_owned(), Json::Bool(false));
    assert!(!logging_is_off(&open, &request, false));
    assert!(logging_is_off(&options(true), &Object::new(), false));
    let message_logging_off = CustomLoggerOptions {
        turn_off_message_logging: false,
        message_logging: false,
    };
    assert!(logging_is_off(&message_logging_off, &Object::new(), false));
}

#[tokio::test]
async fn datadog_llm_obs_embedding_kind_needs_a_caller_parent() {
    let mut payload = support::standard_payload("chatcmpl-1");
    payload.call_type = "embedding".to_owned();
    let without = assemble(&settings(false), &payload, &Object::new(), false)
        .await
        .expect("the span assembles");
    assert_eq!(
        without.meta.expect("the span carries meta").kind,
        Some(SpanKind::Llm)
    );
    let empty = object(&[("parent_id", text(""))]);
    let parentless = assemble(&settings(false), &payload, &empty, false)
        .await
        .expect("the span assembles");
    assert_eq!(
        parentless.meta.expect("the span carries meta").kind,
        Some(SpanKind::Llm)
    );
    let parented = object(&[("parent_id", text("parent"))]);
    let child = assemble(&settings(false), &payload, &parented, false)
        .await
        .expect("the span assembles");
    assert_eq!(
        child.meta.expect("the span carries meta").kind,
        Some(SpanKind::Embedding)
    );
}

#[test]
fn datadog_llm_obs_redaction_headers_and_the_global_flag_are_honored() {
    let open = options(false);
    let disabled = object(&[(
        "headers",
        Json::Object(IndexMap::from([(
            "litellm-disable-message-redaction".to_owned(),
            Json::Bool(true),
        )])),
    )]);
    assert!(!logging_is_off(&open, &disabled, true));
    let enabled = object(&[(
        "headers",
        Json::Object(IndexMap::from([(
            "x-litellm-enable-message-redaction".to_owned(),
            Json::Bool(true),
        )])),
    )]);
    assert!(logging_is_off(&open, &enabled, false));
    assert!(logging_is_off(&open, &Object::new(), true));
    let request_flag = object(&[("turn_off_message_logging", Json::Bool(false))]);
    assert!(!logging_is_off(&open, &request_flag, true));
}

#[tokio::test]
async fn datadog_llm_obs_global_redaction_drops_tool_definitions() {
    let mut init = support::agent_init();
    init.global_turn_off = true;
    let (logger, _transport) = support::build(&init, &support::environment(&[]), Vec::new());
    let mut payload = support::standard_payload("chatcmpl-1");
    payload.model_parameters.insert(
        "tools".to_owned(),
        serde_json::from_str(
            r#"[{"type": "function", "function": {"name": "get_weather", "parameters": {"type": "object"}}}]"#,
        )
        .expect("the tools parse"),
    );
    logger
        .async_log_success_event(support::event(payload))
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
    assert_eq!(message.content, text(REDACTED_TEXT));
}

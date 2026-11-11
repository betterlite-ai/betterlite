use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{
    Fixture, call_kwargs, capture_logs, capture_subscriber, default_fixture, event,
    event_with_dynamic, fallback_kwargs, fallback_response, json, number_value, object, payload,
};
use uuid::Uuid;

use crate::callback_logger::CallbackLogger as _;
use crate::fields::timestamps::Moment;
use crate::newrelic_agent::events::SUMMARY_EVENT_NAME;
use crate::newrelic_agent::success::{SuccessCall, process_success};
use crate::newrelic_extraction::content::RECORD_CONTENT_ENV;
use crate::newrelic_extraction::sources::{CallKwargs, EventSources};

fn summary_attributes(fixture: &Fixture) -> IndexMap<String, Json> {
    fixture.application.events_named(SUMMARY_EVENT_NAME)[0]
        .attributes
        .clone()
}

fn message_attributes(fixture: &Fixture) -> Vec<IndexMap<String, Json>> {
    fixture
        .application
        .events_named("LlmChatCompletionMessage")
        .into_iter()
        .map(|event| event.attributes)
        .collect()
}

#[test]
fn a_success_records_the_summary_event() {
    let fixture = default_fixture();
    let logger = fixture.build();
    logger.log_success_event(&event(payload())).unwrap();
    let summary = summary_attributes(&fixture);
    assert_eq!(
        attribute_value(&summary, "request.model"),
        Json::Text("gpt-4o".to_owned())
    );
    assert_eq!(
        attribute_value(&summary, "response.model"),
        Json::Text("gpt-4o-mini".to_owned())
    );
    assert_eq!(
        attribute_value(&summary, "response.choices.finish_reason"),
        Json::Text("stop".to_owned())
    );
    assert_eq!(
        attribute_value(&summary, "response.number_of_messages"),
        json("2")
    );
    assert_eq!(
        attribute_value(&summary, "vendor"),
        Json::Text("openai".to_owned())
    );
    assert_eq!(
        attribute_value(&summary, "ingest_source"),
        Json::Text("litellm".to_owned())
    );
    assert_eq!(
        attribute_value(&summary, "trace_id"),
        Json::Text("trace-1".to_owned())
    );
    assert_eq!(
        attribute_value(&summary, "id"),
        attribute_value(&summary, "request_id")
    );
}

fn attribute_value(attributes: &IndexMap<String, Json>, name: &str) -> Json {
    attributes
        .get(name)
        .unwrap_or_else(|| panic!("missing attribute {name}"))
        .clone()
}

#[test]
fn the_summary_event_carries_usage_duration_and_request_parameters() {
    let fixture = default_fixture();
    let logger = fixture.build();
    logger.log_success_event(&event(payload())).unwrap();
    let summary = summary_attributes(&fixture);
    assert_eq!(
        attribute_value(&summary, "response.usage.prompt_tokens"),
        json("2")
    );
    assert_eq!(
        attribute_value(&summary, "response.usage.completion_tokens"),
        json("3")
    );
    assert_eq!(
        attribute_value(&summary, "response.usage.total_tokens"),
        json("5")
    );
    assert_eq!(
        number_value(&summary, "duration").to_bits(),
        1_500.0_f64.to_bits()
    );
    assert_eq!(
        attribute_value(&summary, "request.temperature"),
        json("0.5")
    );
    assert_eq!(attribute_value(&summary, "request.max_tokens"), json("100"));
}

#[test]
fn request_messages_carry_ids_sequences_and_content() {
    let fixture = default_fixture();
    let logger = fixture.build();
    logger.log_success_event(&event(payload())).unwrap();
    let messages = message_attributes(&fixture);
    assert_eq!(messages.len(), 2);
    let request = &messages[0];
    assert_eq!(
        attribute_value(request, "id"),
        Json::Text("chatcmpl-1-0".to_owned())
    );
    assert_eq!(
        attribute_value(request, "role"),
        Json::Text("user".to_owned())
    );
    assert_eq!(attribute_value(request, "sequence"), json("0"));
    assert_eq!(
        attribute_value(request, "content"),
        Json::Text("hello".to_owned())
    );
    assert_eq!(attribute_value(request, "token_count"), json("0"));
    assert_eq!(attribute_value(request, "timestamp"), json("1704067200000"));
    assert_eq!(request.get("is_response"), None);
    assert_eq!(
        attribute_value(request, "completion_id"),
        attribute_value(request, "request_id")
    );
}

#[test]
fn response_messages_are_flagged_and_timestamped() {
    let fixture = default_fixture();
    let logger = fixture.build();
    logger.log_success_event(&event(payload())).unwrap();
    let messages = message_attributes(&fixture);
    let response = &messages[1];
    assert_eq!(
        attribute_value(response, "id"),
        Json::Text("chatcmpl-1-1".to_owned())
    );
    assert_eq!(
        attribute_value(response, "role"),
        Json::Text("assistant".to_owned())
    );
    assert_eq!(attribute_value(response, "sequence"), json("1"));
    assert_eq!(attribute_value(response, "is_response"), Json::Bool(true));
    assert_eq!(
        attribute_value(response, "timestamp"),
        json("1704067201500")
    );
    assert_eq!(
        attribute_value(response, "response.model"),
        Json::Text("gpt-4o-mini".to_owned())
    );
    assert_eq!(
        attribute_value(response, "vendor"),
        Json::Text("openai".to_owned())
    );
}

#[test]
fn message_events_carry_the_trace_id() {
    let fixture = default_fixture();
    let logger = fixture.build();
    logger.log_success_event(&event(payload())).unwrap();
    let messages = message_attributes(&fixture);
    assert_eq!(
        attribute_value(&messages[0], "trace_id"),
        Json::Text("trace-1".to_owned())
    );
    assert_eq!(
        attribute_value(&messages[1], "trace_id"),
        Json::Text("trace-1".to_owned())
    );
}

#[test]
fn message_events_share_the_summary_request_id() {
    let fixture = default_fixture();
    let logger = fixture.build();
    logger.log_success_event(&event(payload())).unwrap();
    let summary_id = attribute_value(&summary_attributes(&fixture), "id");
    let messages = message_attributes(&fixture);
    assert_eq!(attribute_value(&messages[0], "request_id"), summary_id);
    assert_eq!(attribute_value(&messages[0], "completion_id"), summary_id);
    assert_eq!(attribute_value(&messages[1], "request_id"), summary_id);
    assert_eq!(attribute_value(&messages[1], "completion_id"), summary_id);
}

#[test]
fn the_summary_id_is_the_generated_request_uuid() {
    let fixture = default_fixture();
    let logger = fixture.build();
    logger.log_success_event(&event(payload())).unwrap();
    let summary = summary_attributes(&fixture);
    assert_eq!(
        attribute_value(&summary, "id"),
        Json::Text(Uuid::from_u128(1).to_string())
    );
    assert_eq!(
        attribute_value(&summary, "trace_id"),
        Json::Text("trace-1".to_owned())
    );
}

#[test]
fn content_is_omitted_when_message_logging_is_turned_off() {
    let mut off = default_fixture();
    off.turn_off(true);
    let logger = off.build();
    logger.log_success_event(&event(payload())).unwrap();
    let messages = message_attributes(&off);
    assert_eq!(messages[0].get("content"), None);
    assert_eq!(messages[1].get("content"), None);
}

#[test]
fn content_is_omitted_when_the_environment_disables_recording() {
    let mut quiet = default_fixture();
    quiet
        .environment
        .insert(RECORD_CONTENT_ENV.to_owned(), "false".to_owned());
    let logger = quiet.build();
    logger.log_success_event(&event(payload())).unwrap();
    let messages = message_attributes(&quiet);
    assert_eq!(messages[0].get("content"), None);
    assert_eq!(messages[1].get("content"), None);
}

#[test]
fn content_is_omitted_when_the_request_asks_for_redaction() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let dynamic = json(r#"{"turn_off_message_logging": true}"#);
    logger
        .log_success_event(&event_with_dynamic(payload(), dynamic))
        .unwrap();
    let messages = message_attributes(&fixture);
    assert_eq!(messages[0].get("content"), None);
    assert_eq!(messages[1].get("content"), None);
}

#[test]
fn a_failed_summary_still_records_the_message_events() {
    let fixture = default_fixture();
    fixture.application.fail_event(SUMMARY_EVENT_NAME);
    let logger = fixture.build();
    let ((), logs) = capture_logs(|| logger.log_success_event(&event(payload())).unwrap());
    assert!(
        fixture
            .application
            .events_named(SUMMARY_EVENT_NAME)
            .is_empty()
    );
    assert_eq!(
        fixture
            .application
            .events_named("LlmChatCompletionMessage")
            .len(),
        2
    );
    assert_eq!(fixture.counter.calls(), vec!["newrelic"]);
    assert!(logs.contains("Failed to record New Relic summary event"));
}

#[tokio::test]
async fn a_malformed_messages_list_counts_a_failure_on_both_success_handlers() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let mut standard = payload();
    standard.messages = Some(json(r#"["hello"]"#));
    let (buffer, _guard) = capture_subscriber();
    logger.log_success_event(&event(standard.clone())).unwrap();
    logger
        .async_log_success_event(event(standard))
        .await
        .unwrap();
    assert_eq!(fixture.counter.calls(), vec!["newrelic", "newrelic"]);
    assert!(fixture.application.events().is_empty());
    assert!(buffer.text().contains("Error in New Relic success event"));
}

#[test]
fn events_are_skipped_with_a_warning_when_the_application_is_disabled() {
    let fixture = default_fixture();
    fixture.application.set_enabled(false);
    let logger = fixture.build();
    let ((), logs) = capture_logs(|| logger.log_success_event(&event(payload())).unwrap());
    assert!(fixture.application.events().is_empty());
    assert!(logs.contains("skipping summary event recording"));
    assert!(logs.contains("skipping message event recording"));
    assert!(fixture.counter.calls().is_empty());
}

#[test]
fn a_call_without_a_standard_payload_uses_the_response_and_times() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let response = fallback_response();
    let kwargs = fallback_kwargs();
    let sources = EventSources {
        kwargs: &kwargs,
        response: &response,
        standard_logging_object: None,
    };
    let call = SuccessCall {
        sources,
        start_time: Some(Moment::Integer(1_704_067_200)),
        end_time: Some(Moment::Integer(1_704_067_202)),
        redact_message_logging: false,
    };
    process_success(&logger, &call).unwrap();
    let summary = summary_attributes(&fixture);
    assert_eq!(
        attribute_value(&summary, "vendor"),
        Json::Text("litellm".to_owned())
    );
    assert_eq!(
        number_value(&summary, "duration").to_bits(),
        2_000.0_f64.to_bits()
    );
    assert_eq!(summary.get("request.temperature"), None);
    assert_eq!(
        attribute_value(&summary, "request.model"),
        Json::Text("gpt-4o".to_owned())
    );
    assert_eq!(
        attribute_value(&summary, "response.choices.finish_reason"),
        Json::Text("length".to_owned())
    );
}

#[test]
fn a_call_without_a_standard_payload_uses_the_response_counts() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let response = fallback_response();
    let kwargs = fallback_kwargs();
    let sources = EventSources {
        kwargs: &kwargs,
        response: &response,
        standard_logging_object: None,
    };
    let call = SuccessCall {
        sources,
        start_time: None,
        end_time: None,
        redact_message_logging: false,
    };
    process_success(&logger, &call).unwrap();
    let summary = summary_attributes(&fixture);
    assert_eq!(
        attribute_value(&summary, "response.number_of_messages"),
        json("1")
    );
    assert_eq!(
        attribute_value(&summary, "response.usage.total_tokens"),
        json("3")
    );
}

#[test]
fn a_call_without_a_standard_payload_records_the_response_message() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let response = fallback_response();
    let kwargs = call_kwargs();
    let sources = EventSources {
        kwargs: &kwargs,
        response: &response,
        standard_logging_object: None,
    };
    let call = SuccessCall {
        sources,
        start_time: None,
        end_time: None,
        redact_message_logging: false,
    };
    process_success(&logger, &call).unwrap();
    let messages = message_attributes(&fixture);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        attribute_value(&messages[0], "id"),
        Json::Text("chatcmpl-9-0".to_owned())
    );
    assert_eq!(
        attribute_value(&messages[0], "content"),
        Json::Text("hi".to_owned())
    );
}

#[test]
fn the_trace_id_comes_from_a_case_insensitive_traceparent_header() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let mut standard = payload();
    standard.trace_id = String::new();
    let headers =
        object(r#"{"TraceParent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0cb902b7-00"}"#);
    let kwargs = CallKwargs {
        headers: Some(&headers),
        ..call_kwargs()
    };
    let empty = IndexMap::new();
    let sources = EventSources {
        kwargs: &kwargs,
        response: &empty,
        standard_logging_object: Some(&standard),
    };
    let call = SuccessCall {
        sources,
        start_time: None,
        end_time: None,
        redact_message_logging: false,
    };
    process_success(&logger, &call).unwrap();
    let summary = summary_attributes(&fixture);
    assert_eq!(
        attribute_value(&summary, "trace_id"),
        Json::Text("4bf92f3577b34da6a3ce929d0e0e4736".to_owned())
    );
}

#[test]
fn a_generated_trace_id_is_a_simple_uuid_when_nothing_is_available() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let mut standard = payload();
    standard.trace_id = String::new();
    let kwargs = call_kwargs();
    let empty = IndexMap::new();
    let sources = EventSources {
        kwargs: &kwargs,
        response: &empty,
        standard_logging_object: Some(&standard),
    };
    let call = SuccessCall {
        sources,
        start_time: None,
        end_time: None,
        redact_message_logging: false,
    };
    process_success(&logger, &call).unwrap();
    let summary = summary_attributes(&fixture);
    assert_eq!(
        attribute_value(&summary, "trace_id"),
        Json::Text("00000000000000000000000000000001".to_owned())
    );
}

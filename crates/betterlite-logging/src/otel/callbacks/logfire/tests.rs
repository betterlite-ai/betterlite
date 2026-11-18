#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;

use super::{
    CHAT_COMPLETION_TEMPLATE, DEFAULT_TEMPLATE, EMBEDDING_TEMPLATE, IMAGE_GENERATION_TEMPLATE,
    LogfireLevel, LogfireLogger, LogfireSink, SpanConfig,
};
use crate::callback_logger::CallbackLogger as _;
use crate::otel::callbacks::IdGenerator;
use crate::otel::callbacks::tests::{
    DEFAULT_MESSAGES, DEFAULT_MODEL_PARAMETERS, SUCCESS_RESPONSE, event, event_with_metadata,
    from_payload, json, payload,
};

#[derive(Clone, Debug)]
struct Record {
    level: LogfireLevel,
    template: &'static str,
    request_data: Json,
    exc_info: Option<bool>,
}

#[derive(Clone, Default)]
struct RecordingSink {
    records: Arc<Mutex<Vec<Record>>>,
}

impl RecordingSink {
    fn recorded(&self) -> Vec<Record> {
        self.records
            .lock()
            .expect("the recorded spans lock")
            .clone()
    }

    fn record(&self, level: LogfireLevel, config: &SpanConfig<'_>, exc_info: Option<bool>) {
        let entry = Record {
            level,
            template: config.message_template,
            request_data: config.span_data.request_data.clone(),
            exc_info,
        };
        self.records
            .lock()
            .expect("the recorded spans lock")
            .push(entry);
    }
}

impl LogfireSink for RecordingSink {
    fn info(&self, config: &SpanConfig<'_>) {
        self.record(LogfireLevel::Info, config, None);
    }

    fn error(&self, config: &SpanConfig<'_>, exc_info: bool) {
        self.record(LogfireLevel::Error, config, Some(exc_info));
    }
}

// `info` and `error` are one-line because they only pick the level; `record` holds the shared body
fn generated_id() -> String {
    "generated-id".to_owned()
}

fn unused_id() -> String {
    "unused-id".to_owned()
}

fn logger(sink: &RecordingSink, ids: IdGenerator, redact: bool) -> LogfireLogger {
    LogfireLogger::new(Box::new(sink.clone()), ids, redact)
}

fn field<'value>(value: &'value Json, name: &str) -> &'value Json {
    let Json::Object(fields) = value else {
        panic!("the request data is an object");
    };
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the request data carries {name}"))
}

fn seconds(value: &Json) -> i64 {
    let Json::Text(text) = value else {
        panic!("the timestamp is RFC 3339 text");
    };
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 timestamp")
        .timestamp()
}

fn payload_for_call_type(call_type: &str) -> String {
    payload(SUCCESS_RESPONSE, DEFAULT_MESSAGES).replace(
        r#""call_type": "completion""#,
        &format!(r#""call_type": "{call_type}""#),
    )
}

#[rstest]
#[case::completion("completion", CHAT_COMPLETION_TEMPLATE)]
#[case::async_completion("acompletion", CHAT_COMPLETION_TEMPLATE)]
#[case::embedding("embedding", EMBEDDING_TEMPLATE)]
#[case::async_embedding("aembedding", EMBEDDING_TEMPLATE)]
#[case::image_generation("image_generation", IMAGE_GENERATION_TEMPLATE)]
#[case::async_image_generation("aimage_generation", IMAGE_GENERATION_TEMPLATE)]
#[case::other("text_completion", DEFAULT_TEMPLATE)]
fn logfire_selects_the_message_template_for_each_call_type(
    #[case] call_type: &str,
    #[case] template: &'static str,
) {
    let sink = RecordingSink::default();
    let logger = logger(&sink, generated_id, false);
    logger
        .log_success_event(&from_payload(&payload_for_call_type(call_type)))
        .expect("the event is logged");
    let records = sink.recorded();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].template, template);
}

#[test]
fn logfire_logs_a_success_at_info() {
    let sink = RecordingSink::default();
    let logger = logger(&sink, generated_id, false);
    logger
        .log_success_event(&event())
        .expect("the event is logged");
    let records = sink.recorded();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].level, LogfireLevel::Info);
    assert_eq!(records[0].exc_info, None);
}

#[test]
fn logfire_logs_a_failure_at_error_with_exc_info() {
    let sink = RecordingSink::default();
    let logger = logger(&sink, generated_id, false);
    logger
        .log_failure_event(&event())
        .expect("the event is logged");
    let records = sink.recorded();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].level, LogfireLevel::Error);
    assert_eq!(records[0].exc_info, Some(true));
}

#[test]
fn logfire_sends_the_request_half_of_the_request_data() {
    let sink = RecordingSink::default();
    let logger = logger(&sink, unused_id, false);
    logger
        .log_success_event(&event())
        .expect("the event is logged");
    let records = sink.recorded();
    let data = &records[0].request_data;
    assert_eq!(
        field(data, "id"),
        &Json::Text("chatcmpl-response-1".to_owned())
    );
    assert_eq!(
        field(data, "call_type"),
        &Json::Text("completion".to_owned())
    );
    assert_eq!(field(data, "cache_hit"), &Json::Bool(false));
    assert_eq!(field(data, "model"), &Json::Text("gpt-4o".to_owned()));
    assert_eq!(field(data, "user"), &Json::Text("end-user-1".to_owned()));
    assert_eq!(field(data, "spend"), &json("0.0000125"));
    assert_eq!(field(data, "responseTime (seconds)"), &json("1.5"));
    assert_eq!(
        field(data, "modelParameters"),
        &json(DEFAULT_MODEL_PARAMETERS)
    );
    assert_eq!(field(data, "metadata"), &Json::Object(IndexMap::new()));
    assert_eq!(seconds(field(data, "startTime")), 1_704_067_200);
    assert_eq!(seconds(field(data, "endTime")), 1_704_067_201);
}

#[test]
fn logfire_sends_the_response_half_of_the_request_data() {
    let sink = RecordingSink::default();
    let logger = logger(&sink, unused_id, false);
    logger
        .log_success_event(&event())
        .expect("the event is logged");
    let records = sink.recorded();
    let data = &records[0].request_data;
    assert_eq!(field(data, "messages"), &json(DEFAULT_MESSAGES));
    assert_eq!(field(data, "response"), &json(SUCCESS_RESPONSE));
    assert_eq!(
        field(data, "usage"),
        &json(r#"{"total_tokens": 11, "completion_tokens": 3, "prompt_tokens": 8}"#)
    );
}

#[test]
fn logfire_prefers_the_response_id() {
    let sink = RecordingSink::default();
    let logger = logger(&sink, unused_id, false);
    logger
        .log_success_event(&event())
        .expect("the event is logged");
    let records = sink.recorded();
    assert_eq!(
        field(&records[0].request_data, "id"),
        &Json::Text("chatcmpl-response-1".to_owned())
    );
}

#[test]
fn logfire_generates_an_id_when_the_response_has_none() {
    let sink = RecordingSink::default();
    let logger = logger(&sink, generated_id, false);
    let event = from_payload(&payload(
        r#"{"model": "gpt-4o-2024-08-06"}"#,
        DEFAULT_MESSAGES,
    ));
    logger
        .log_success_event(&event)
        .expect("the event is logged");
    let records = sink.recorded();
    assert_eq!(
        field(&records[0].request_data, "id"),
        &Json::Text("generated-id".to_owned())
    );
}

#[test]
fn logfire_defaults_a_missing_response_to_an_empty_object() {
    let sink = RecordingSink::default();
    let logger = logger(&sink, generated_id, false);
    let event = from_payload(&payload("null", "null"));
    logger
        .log_success_event(&event)
        .expect("the event is logged");
    let records = sink.recorded();
    let data = &records[0].request_data;
    assert_eq!(field(data, "response"), &Json::Object(IndexMap::new()));
    assert_eq!(field(data, "usage"), &Json::Object(IndexMap::new()));
    assert_eq!(field(data, "messages"), &Json::Null);
}

#[test]
fn logfire_excludes_the_three_metadata_keys() {
    let sink = RecordingSink::default();
    let logger = logger(&sink, generated_id, false);
    let metadata = IndexMap::from([
        (
            "endpoint".to_owned(),
            Json::Text("https://api.openai.com".to_owned()),
        ),
        ("caching_groups".to_owned(), Json::List(Vec::new())),
        ("previous_models".to_owned(), Json::Null),
        ("team".to_owned(), Json::Text("core".to_owned())),
    ]);
    let event = event_with_metadata(&payload(SUCCESS_RESPONSE, DEFAULT_MESSAGES), metadata);
    logger
        .log_success_event(&event)
        .expect("the event is logged");
    let records = sink.recorded();
    assert_eq!(
        field(&records[0].request_data, "metadata"),
        &Json::Object(IndexMap::from([(
            "team".to_owned(),
            Json::Text("core".to_owned())
        )]))
    );
}

#[test]
fn logfire_redacts_user_api_key_metadata_when_asked() {
    let sink = RecordingSink::default();
    let logger = logger(&sink, generated_id, true);
    let metadata = IndexMap::from([
        (
            "user_api_key_hash".to_owned(),
            Json::Text("hash".to_owned()),
        ),
        (
            "user_api_key_alias".to_owned(),
            Json::Text("alias".to_owned()),
        ),
        ("team".to_owned(), Json::Text("core".to_owned())),
    ]);
    let event = event_with_metadata(&payload(SUCCESS_RESPONSE, DEFAULT_MESSAGES), metadata);
    logger
        .log_success_event(&event)
        .expect("the event is logged");
    let records = sink.recorded();
    assert_eq!(
        field(&records[0].request_data, "metadata"),
        &Json::Object(IndexMap::from([(
            "team".to_owned(),
            Json::Text("core".to_owned())
        )]))
    );
}

#[test]
fn logfire_keeps_user_api_key_metadata_when_redaction_is_off() {
    let sink = RecordingSink::default();
    let logger = logger(&sink, generated_id, false);
    let metadata = IndexMap::from([(
        "user_api_key_hash".to_owned(),
        Json::Text("hash".to_owned()),
    )]);
    let event = event_with_metadata(&payload(SUCCESS_RESPONSE, DEFAULT_MESSAGES), metadata);
    logger
        .log_success_event(&event)
        .expect("the event is logged");
    let records = sink.recorded();
    assert_eq!(
        field(&records[0].request_data, "metadata"),
        &Json::Object(IndexMap::from([(
            "user_api_key_hash".to_owned(),
            Json::Text("hash".to_owned())
        )]))
    );
}

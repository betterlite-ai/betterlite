#![cfg(test)]

use std::fs::read;
use std::path::PathBuf;
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rstest::rstest;

use super::{
    Chunk, StreamFieldError, extract_message_start_field, resolve_logged_model, split_lines,
};

const MODEL_EVENT: &str =
    r#"data: {"type": "message_start", "message": {"id": "msg_1", "model": "chunk-model"}}"#;

#[test]
fn fields_extract_returns_the_first_non_empty_string() {
    let chunks = [
        Chunk::Text(format!("event: message_start\n{MODEL_EVENT}\n\n")),
        Chunk::Bytes(MODEL_EVENT.as_bytes().to_vec()),
    ];
    let found = extract_message_start_field(&chunks, "model").expect("valid text");
    assert_eq!(found, Some("chunk-model".to_owned()));
}

#[test]
fn fields_extract_skips_bad_json_and_other_message_types() {
    let text = concat!(
        "data: {not json}\n",
        "data: [1, 2]\n",
        "data: \"text\"\n",
        "data: {\"type\": \"content_block_delta\", \"model\": \"ignored\"}\n",
        "data: {\"message\": {\"model\": \"ignored\"}}\n",
        " data: {\"type\": \"message_start\", \"message\": {\"model\": \"indented\"}}\n",
        "data {\"type\": \"message_start\", \"message\": {\"model\": \"colonless\"}}\n",
    );
    let chunks = [Chunk::Text(text.to_owned())];
    assert_eq!(
        extract_message_start_field(&chunks, "model").expect("valid text"),
        None
    );
}

#[test]
fn fields_extract_keeps_scanning_past_a_missing_field() {
    let text = concat!(
        "data: {\"type\": \"message_start\", \"message\": {\"id\": \"msg_1\"}}\n",
        "data: {\"type\": \"message_start\", \"message\": {\"model\": 7}}\n",
        "data: {\"type\": \"message_start\", \"message\": {\"model\": \"\"}}\n",
        "data: {\"type\": \"message_start\", \"message\": {\"model\": null}}\n",
    );
    let chunks = [
        Chunk::Text(text.to_owned()),
        Chunk::Text(format!("{MODEL_EVENT}\n")),
    ];
    assert_eq!(
        extract_message_start_field(&chunks, "model").expect("valid text"),
        Some("chunk-model".to_owned())
    );
}

#[test]
fn fields_extract_skips_a_non_object_message() {
    let text = concat!(
        "data: {\"type\": \"message_start\", \"message\": \"text\"}\n",
        "data: {\"type\": \"message_start\", \"message\": [1]}\n",
        "data: {\"type\": \"message_start\", \"message\": null}\n",
        "data: {\"type\": \"message_start\"}\n",
    );
    let chunks = [Chunk::Text(text.to_owned())];
    assert_eq!(
        extract_message_start_field(&chunks, "model").expect("valid text"),
        None
    );
}

#[rstest]
#[case("\n")]
#[case("\r")]
#[case("\r\n")]
#[case("\u{0b}")]
#[case("\u{0c}")]
#[case("\u{1c}")]
#[case("\u{1d}")]
#[case("\u{1e}")]
#[case("\u{85}")]
#[case("\u{2028}")]
#[case("\u{2029}")]
fn fields_extract_splits_on_every_splitlines_boundary(#[case] separator: &str) {
    let text = format!("padding{separator}{MODEL_EVENT}{separator}");
    let chunks = [Chunk::Text(text)];
    assert_eq!(
        extract_message_start_field(&chunks, "model").expect("valid text"),
        Some("chunk-model".to_owned())
    );
}

#[test]
fn fields_split_lines_matches_python_splitlines() {
    assert_eq!(split_lines("a\r\nb"), vec!["a", "b"]);
    assert_eq!(split_lines("a\rb"), vec!["a", "b"]);
    assert_eq!(split_lines("a\n"), vec!["a"]);
    assert_eq!(split_lines("a\n\n"), vec!["a", ""]);
    assert_eq!(split_lines("a\u{2028}b\u{2029}"), vec!["a", "b"]);
    assert_eq!(split_lines(""), Vec::<&str>::new());
    assert_eq!(split_lines("no boundary"), vec!["no boundary"]);
}

#[test]
fn fields_extract_trims_python_whitespace_around_the_data() {
    let text = concat!(
        "data: \u{1c}\t{\"type\": \"message_start\", \"message\": {\"model\": \"a\"}}\u{1f}\u{3000}\n",
        "data:\u{a0}{\"type\": \"message_start\", \"message\": {\"model\": \"b\"}}\n",
    );
    let chunks = [Chunk::Text(text.to_owned())];
    assert_eq!(
        extract_message_start_field(&chunks, "model").expect("valid text"),
        Some("a".to_owned())
    );
}

#[test]
fn fields_extract_decodes_bytes_and_text_alike() {
    let chunks = [
        Chunk::Bytes(b"event: message_start\n".to_vec()),
        Chunk::Text(format!("{MODEL_EVENT}\n")),
    ];
    assert_eq!(
        extract_message_start_field(&chunks, "id").expect("valid text"),
        Some("msg_1".to_owned())
    );
}

#[test]
fn fields_extract_reports_invalid_utf8() {
    let chunks = [
        Chunk::Bytes(b"event: message_start\n".to_vec()),
        Chunk::Bytes(vec![b'x', 0xff]),
    ];
    let error = extract_message_start_field(&chunks, "model").expect_err("invalid bytes");
    let StreamFieldError::Utf8 { index, source } = error;
    assert_eq!(index, 1);
    assert_eq!(source.valid_up_to(), 1);
}

#[test]
fn fields_extract_does_not_decode_a_later_invalid_chunk() {
    let chunks = [
        Chunk::Text(format!("{MODEL_EVENT}\n")),
        Chunk::Bytes(vec![0xff]),
    ];
    assert_eq!(
        extract_message_start_field(&chunks, "model").expect("the answer comes first"),
        Some("chunk-model".to_owned())
    );
}

#[test]
fn fields_extract_returns_none_without_an_answer() {
    let chunks = [
        Chunk::Text(String::new()),
        Chunk::Text("event: ping\n\n".to_owned()),
        Chunk::Bytes(b"data: {\"type\": \"message_stop\"}\n".to_vec()),
    ];
    assert_eq!(
        extract_message_start_field(&chunks, "model").expect("valid text"),
        None
    );
    assert_eq!(
        extract_message_start_field(&[], "model").expect("no chunks"),
        None
    );
}

#[test]
fn fields_resolve_logged_model_prefers_the_request_body() {
    let call_log = logging_state(&[("model", Json::Text("call-log-model".to_owned()))], &[]);
    let request = body(r#"{"model": "request-model"}"#);
    let chunks = [Chunk::Text(format!("{MODEL_EVENT}\n"))];
    assert_eq!(
        resolve_logged_model(&call_log, &request, &chunks).expect("valid text"),
        "request-model"
    );
}

#[rstest]
#[case("{}")]
#[case(r#"{"model": ""}"#)]
#[case(r#"{"model": 7}"#)]
#[case(r#"{"model": null}"#)]
fn fields_resolve_logged_model_uses_the_call_log_without_a_request_model(#[case] request: &str) {
    let call_log = logging_state(&[("model", Json::Text("call-log-model".to_owned()))], &[]);
    let chunks = [Chunk::Text(format!("{MODEL_EVENT}\n"))];
    assert_eq!(
        resolve_logged_model(&call_log, &body(request), &chunks).expect("valid text"),
        "call-log-model"
    );
}

#[test]
fn fields_resolve_logged_model_falls_to_the_chunk_model() {
    let call_log = logging_state(&[("model", Json::Text("unknown".to_owned()))], &[]);
    let request = body(r#"{"model": "unknown"}"#);
    let chunks = [Chunk::Text(format!("{MODEL_EVENT}\n"))];
    assert_eq!(
        resolve_logged_model(&call_log, &request, &chunks).expect("valid text"),
        "chunk-model"
    );
}

#[test]
fn fields_resolve_logged_model_falls_to_the_chunk_model_without_a_logged_model() {
    let call_log = logging_state(&[], &[]);
    let chunks = [Chunk::Text(format!("{MODEL_EVENT}\n"))];
    assert_eq!(
        resolve_logged_model(&call_log, &body("{}"), &chunks).expect("valid text"),
        "chunk-model"
    );
}

#[test]
fn fields_resolve_logged_model_returns_the_first_value_without_a_chunk_model() {
    let call_log = logging_state(&[("model", Json::Text("unknown".to_owned()))], &[]);
    let chunks = [Chunk::Text("event: ping\n".to_owned())];
    assert_eq!(
        resolve_logged_model(&call_log, &body("{}"), &chunks).expect("valid text"),
        "unknown"
    );
    let empty = logging_state(&[], &[]);
    assert_eq!(
        resolve_logged_model(&empty, &body("{}"), &chunks).expect("valid text"),
        ""
    );
}

#[test]
fn fields_resolve_logged_model_propagates_a_utf8_error() {
    let call_log = logging_state(&[], &[]);
    let chunks = [Chunk::Bytes(vec![0xff])];
    let error = resolve_logged_model(&call_log, &body(r#"{"model": "unknown"}"#), &chunks)
        .expect_err("invalid bytes");
    assert!(matches!(error, StreamFieldError::Utf8 { index: 0, .. }));
}

#[rstest]
fn replays_the_message_start_field_corpus(
    #[files("fuzz/corpus/message_start_field/*")] path: PathBuf,
) {
    let bytes = read(&path).expect("a readable corpus seed");
    let chunks = [Chunk::Bytes(bytes)];
    if let Ok(Some(value)) = extract_message_start_field(&chunks, "model") {
        assert!(!value.is_empty(), "{} gave an empty model", path.display());
    }
    if let Ok(Some(value)) = extract_message_start_field(&chunks, "id") {
        assert!(!value.is_empty(), "{} gave an empty id", path.display());
    }
}

fn body(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("a JSON object")
}

fn timestamp() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").expect("a valid timestamp")
}

fn logging_state(details: &[(&str, Json)], litellm_params: &[(&str, Json)]) -> LoggingState {
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut state = LoggingState::new(
        LoggingInit {
            model: String::new(),
            call_type: "pass_through_endpoint".to_owned(),
            start_time: timestamp(),
            litellm_call_id: "call-1".to_owned(),
            function_id: "function-1".to_owned(),
            messages: None,
            stream: Some(false),
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: params(litellm_params),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state");
    for (name, value) in details {
        state
            .model_call_details
            .insert((*name).to_owned(), value.clone());
    }
    state
}

fn params(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    let mut map = IndexMap::new();
    for (name, value) in entries {
        map.insert((*name).to_owned(), value.clone());
    }
    map
}

#![cfg(test)]

use std::sync::Mutex;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::base64_strip::{MAX_DEPTH_REACHED, strip_base64_from_messages};
use super::failure_counter::{CallbackFailureCounter, handle_callback_failure};
use super::health::{HealthState, IntegrationHealthStatus};
use super::metadata_field::{MetadataField, select_metadata_field};
use super::options::CustomLoggerOptions;
use super::redaction::{
    ModelCallDetails, Redaction, RedactionPolicy, ResponseStamp, redact_standard_logging_payload,
};
use super::truncation::{
    MAX_LOGGED_TEXT_CHARS, TRUNCATION_SUFFIX, truncate_standard_logging_payload_content,
};
use crate::fields::payload::StandardLoggingPayload;

const PAYLOAD: &str = r#"{
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
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).expect("a valid standard logging payload")
}

fn payload_with(field: &str, value: Json) -> StandardLoggingPayload {
    let mut raw: Json = serde_json::from_str(PAYLOAD).expect("a valid payload object");
    let Json::Object(fields) = &mut raw else {
        panic!("the payload is an object");
    };
    fields.insert(field.to_owned(), value);
    let source = serde_json::to_string(&raw).expect("the payload encodes");
    serde_json::from_str(&source).expect("the payload parses")
}

fn encoded(payload: &StandardLoggingPayload) -> String {
    serde_json::to_string(payload).expect("the payload encodes")
}

#[test]
fn options_default_to_logging_messages() {
    let options: CustomLoggerOptions = serde_json::from_str("{}").expect("the defaults parse");
    assert_eq!(
        options,
        CustomLoggerOptions {
            turn_off_message_logging: false,
            message_logging: true,
        }
    );
    assert_eq!(CustomLoggerOptions::default(), options);
}

#[test]
fn options_read_both_flags_and_ignore_unrelated_keys() {
    let options: CustomLoggerOptions = serde_json::from_str(
        r#"{"turn_off_message_logging": true, "message_logging": false, "dd_api_key": "secret"}"#,
    )
    .expect("the options parse");
    assert!(options.turn_off_message_logging);
    assert!(!options.message_logging);
}

#[test]
fn options_read_a_null_turn_off_message_logging_as_false() {
    let options: CustomLoggerOptions =
        serde_json::from_str(r#"{"turn_off_message_logging": null}"#).expect("a null flag parses");
    assert!(!options.turn_off_message_logging);
    assert!(options.message_logging);
}

#[rstest]
#[case::turn_off(r#"{"turn_off_message_logging": "yes"}"#)]
#[case::message_logging(r#"{"message_logging": "yes"}"#)]
fn options_reject_a_flag_that_is_not_a_bool(#[case] source: &str) {
    let _error = serde_json::from_str::<CustomLoggerOptions>(source)
        .expect_err("a non-bool flag is rejected");
}

#[test]
fn truncate_cuts_a_long_string_to_the_limit_and_suffix() {
    let long = "a".repeat(MAX_LOGGED_TEXT_CHARS + 1);
    let truncated =
        truncate_standard_logging_payload_content(&payload_with("error_str", Json::Text(long)));
    let mut expected = "a".repeat(MAX_LOGGED_TEXT_CHARS);
    expected.push_str(TRUNCATION_SUFFIX);
    assert_eq!(truncated.error_str, Some(expected));
}

#[test]
fn truncate_keeps_a_field_of_exactly_the_limit() {
    let exact = "a".repeat(MAX_LOGGED_TEXT_CHARS);
    let truncated = truncate_standard_logging_payload_content(&payload_with(
        "error_str",
        Json::Text(exact.clone()),
    ));
    assert_eq!(truncated.error_str, Some(exact));
}

#[test]
fn truncate_counts_unicode_characters_not_bytes() {
    let long = "\u{e9}".repeat(MAX_LOGGED_TEXT_CHARS + 1);
    let truncated =
        truncate_standard_logging_payload_content(&payload_with("error_str", Json::Text(long)));
    let text = truncated.error_str.expect("the field stays a string");
    assert_eq!(
        text.chars().count(),
        MAX_LOGGED_TEXT_CHARS + TRUNCATION_SUFFIX.len()
    );
    assert!(text.ends_with(TRUNCATION_SUFFIX));
}

#[test]
fn truncate_leaves_absent_and_null_fields_alone() {
    let truncated = truncate_standard_logging_payload_content(&payload());
    assert_eq!(truncated.error_str, None);
    assert_eq!(truncated.messages, None);
    assert_eq!(truncated.response, None);
}

#[rstest]
#[case::empty_string("error_str", Json::Text(String::new()))]
#[case::empty_list("messages", Json::List(Vec::new()))]
#[case::empty_object("response", Json::Object(IndexMap::new()))]
fn truncate_keeps_an_empty_field_with_its_type(#[case] field: &str, #[case] value: Json) {
    let truncated = truncate_standard_logging_payload_content(&payload_with(field, value.clone()));
    assert_eq!(field_of(&truncated, field), value);
}

#[test]
fn truncate_keeps_a_short_list_and_object_with_their_types() {
    let messages = Json::List(vec![Json::Text("hi".to_owned())]);
    let response = Json::Object(IndexMap::from([(
        "choices".to_owned(),
        Json::List(Vec::new()),
    )]));
    let truncated_messages =
        truncate_standard_logging_payload_content(&payload_with("messages", messages.clone()));
    assert_eq!(truncated_messages.messages, Some(messages));
    let truncated_response =
        truncate_standard_logging_payload_content(&payload_with("response", response.clone()));
    assert_eq!(truncated_response.response, Some(response));
}

#[test]
fn truncate_cuts_a_long_list_to_its_json_text() {
    let long = Json::List(
        (0_u32..3_000)
            .map(|index| Json::Number(serde_json::Number::from(index)))
            .collect(),
    );
    let truncated = truncate_standard_logging_payload_content(&payload_with("messages", long));
    let Some(Json::Text(text)) = truncated.messages else {
        panic!("a long list becomes a string");
    };
    assert_eq!(
        text.chars().count(),
        MAX_LOGGED_TEXT_CHARS + TRUNCATION_SUFFIX.len()
    );
    assert!(text.ends_with(TRUNCATION_SUFFIX));
}

#[test]
fn truncate_ignores_fields_beyond_the_three() {
    let long = "a".repeat(MAX_LOGGED_TEXT_CHARS + 1);
    let truncated =
        truncate_standard_logging_payload_content(&payload_with("id", Json::Text(long.clone())));
    assert_eq!(truncated.id, long);
}

#[test]
fn truncate_leaves_the_input_payload_alone() {
    let long = "a".repeat(MAX_LOGGED_TEXT_CHARS + 1);
    let original = payload_with("error_str", Json::Text(long.clone()));
    let before = encoded(&original);
    let truncated = truncate_standard_logging_payload_content(&original);
    assert_eq!(encoded(&original), before);
    assert_eq!(original.error_str, Some(long));
    assert_ne!(encoded(&truncated), before);
}

fn field_of(payload: &StandardLoggingPayload, field: &str) -> Json {
    match field {
        "error_str" => payload
            .error_str
            .clone()
            .map(Json::Text)
            .expect("the field is set"),
        "messages" => payload.messages.clone().expect("the field is set"),
        "response" => payload.response.clone().expect("the field is set"),
        _ => panic!("the field is one of the three"),
    }
}

fn kwargs(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).expect("a kwargs object")
}

#[test]
fn metadata_field_is_absent_without_kwargs() {
    assert_eq!(select_metadata_field(None), None);
}

#[rstest]
#[case::empty("{}", MetadataField::Metadata)]
#[case::litellm_metadata_present(r#"{"litellm_metadata": null}"#, MetadataField::LitellmMetadata)]
#[case::metadata_only(r#"{"metadata": {"user": "x"}}"#, MetadataField::Metadata)]
#[case::both(
    r#"{"litellm_metadata": {}, "metadata": {}}"#,
    MetadataField::LitellmMetadata
)]
fn metadata_field_selects_the_key_that_is_present(
    #[case] source: &str,
    #[case] expected: MetadataField,
) {
    assert_eq!(select_metadata_field(Some(&kwargs(source))), Some(expected));
}

#[test]
fn metadata_field_spells_the_keys_as_litellm_does() {
    assert_eq!(MetadataField::LitellmMetadata.as_str(), "litellm_metadata");
    assert_eq!(MetadataField::Metadata.as_str(), "metadata");
    assert_eq!(
        serde_json::to_string(&MetadataField::LitellmMetadata).expect("the field serializes"),
        "\"litellm_metadata\""
    );
}

fn details(
    standard_logging_object: Option<&str>,
    proxy_server_request: Option<&str>,
) -> ModelCallDetails {
    ModelCallDetails {
        standard_logging_object: standard_logging_object
            .map(|source| serde_json::from_str(source).expect("a standard logging object")),
        proxy_server_request: proxy_server_request
            .map(|source| serde_json::from_str(source).expect("a proxy server request")),
    }
}

fn policy(turn_off_message_logging: bool, excluded_fields: &[&str]) -> RedactionPolicy {
    RedactionPolicy {
        turn_off_message_logging,
        excluded_fields: excluded_fields
            .iter()
            .map(|field| (*field).to_owned())
            .collect(),
        callback_redacts_itself: false,
    }
}

fn stamp() -> ResponseStamp {
    ResponseStamp {
        id: "chatcmpl-stamp".to_owned(),
        created: 1_704_067_200,
    }
}

fn redacted(redaction: Redaction) -> (IndexMap<String, Json>, Option<IndexMap<String, Json>>) {
    match redaction {
        Redaction::Redacted {
            standard_logging_object,
            proxy_server_request,
        } => (standard_logging_object, proxy_server_request),
        Redaction::Unchanged => panic!("the details are redacted"),
    }
}

#[test]
fn redact_is_unchanged_without_the_flags() {
    let details = details(Some(r#"{"messages": []}"#), Some(r#"{"url": "http://x"}"#));
    assert_eq!(
        redact_standard_logging_payload(&details, &policy(false, &[]), &stamp()),
        Redaction::Unchanged
    );
}

#[test]
fn redact_is_unchanged_without_a_standard_logging_object() {
    let details = details(None, Some(r#"{"url": "http://x"}"#));
    assert_eq!(
        redact_standard_logging_payload(&details, &policy(true, &["messages"]), &stamp()),
        Redaction::Unchanged
    );
}

#[test]
fn redact_removes_excluded_fields_from_the_copy() {
    let details = details(
        Some(r#"{"id": "s", "messages": [], "response": {"choices": []}, "error_str": "e"}"#),
        None,
    );
    let (object, _) = redacted(redact_standard_logging_payload(
        &details,
        &policy(false, &["messages", "error_str"]),
        &stamp(),
    ));
    assert_eq!(
        object.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["id", "response"]
    );
}

#[rstest]
#[case::turn_off(true)]
#[case::excluded_only(false)]
fn redact_drops_the_classifier_audit_fields_only_with_turn_off(#[case] turn_off: bool) {
    let details = details(
        Some(r#"{"classifier_input": {}, "originating_request_masked": {}, "id": "s"}"#),
        None,
    );
    let (object, _) = redacted(redact_standard_logging_payload(
        &details,
        &policy(turn_off, &["unused"]),
        &stamp(),
    ));
    assert_eq!(object.contains_key("classifier_input"), !turn_off);
    assert_eq!(object.contains_key("originating_request_masked"), !turn_off);
    assert!(object.contains_key("id"));
}

#[test]
fn redact_replaces_messages_with_the_placeholder() {
    let details = details(
        Some(r#"{"messages": [{"role": "user", "content": "hi"}]}"#),
        None,
    );
    let (object, _) = redacted(redact_standard_logging_payload(
        &details,
        &policy(true, &[]),
        &stamp(),
    ));
    let placeholder = serde_json::from_str::<Json>(
        r#"[{"content": "redacted-by-litellm", "role": "assistant", "tool_calls": null, "function_call": null, "provider_specific_fields": null}]"#,
    )
    .expect("the placeholder");
    assert_eq!(object.get("messages"), Some(&placeholder));
}

#[test]
fn redact_keeps_null_and_excluded_content_fields_alone() {
    let both_null = details(Some(r#"{"messages": null, "response": null}"#), None);
    let (object, _) = redacted(redact_standard_logging_payload(
        &both_null,
        &policy(true, &[]),
        &stamp(),
    ));
    assert_eq!(object.get("messages"), Some(&Json::Null));
    assert_eq!(object.get("response"), Some(&Json::Null));
    let excluded = details(
        Some(r#"{"messages": [{"role": "user"}], "response": {"choices": []}}"#),
        None,
    );
    let (excluded_object, _) = redacted(redact_standard_logging_payload(
        &excluded,
        &policy(true, &["messages", "response"]),
        &stamp(),
    ));
    assert!(!excluded_object.contains_key("messages"));
    assert!(!excluded_object.contains_key("response"));
}

#[test]
fn redact_replaces_a_chat_response_with_the_stamped_placeholder() {
    let details = details(
        Some(r#"{"response": {"choices": [{"message": {}}]}}"#),
        None,
    );
    let (object, _) = redacted(redact_standard_logging_payload(
        &details,
        &policy(true, &[]),
        &stamp(),
    ));
    let placeholder = serde_json::from_str::<Json>(
        r#"{"id": "chatcmpl-stamp", "created": 1704067200, "model": null, "object": "chat.completion", "system_fingerprint": null, "choices": [{"finish_reason": "stop", "index": 0, "message": {"content": "redacted-by-litellm", "role": "assistant", "tool_calls": null, "function_call": null}}]}"#,
    )
    .expect("the placeholder");
    assert_eq!(object.get("response"), Some(&placeholder));
}

#[test]
fn redact_keeps_a_responses_api_response_and_redacts_only_its_text() {
    let details = details(
        Some(
            r#"{"response": {"id": "resp-1", "output": [{"type": "message", "content": [{"type": "output_text", "text": "secret"}, {"type": "refusal", "refusal": "no"}]}, {"type": "reasoning", "content": [{"type": "reasoning_text", "text": "why"}]}]}}"#,
        ),
        None,
    );
    let (object, _) = redacted(redact_standard_logging_payload(
        &details,
        &policy(true, &[]),
        &stamp(),
    ));
    let expected = serde_json::from_str::<Json>(
        r#"{"id": "resp-1", "output": [{"type": "message", "content": [{"type": "output_text", "text": "redacted-by-litellm"}, {"type": "refusal", "refusal": "no"}]}, {"type": "reasoning", "content": [{"type": "reasoning_text", "text": "redacted-by-litellm"}]}]}"#,
    )
    .expect("the response");
    assert_eq!(object.get("response"), Some(&expected));
}

#[test]
fn redact_keeps_a_responses_output_that_is_not_a_list() {
    let details = details(
        Some(r#"{"response": {"output": "x", "id": "resp-1"}}"#),
        None,
    );
    let (object, _) = redacted(redact_standard_logging_payload(
        &details,
        &policy(true, &[]),
        &stamp(),
    ));
    let expected: Json =
        serde_json::from_str(r#"{"output": "x", "id": "resp-1"}"#).expect("the response");
    assert_eq!(object.get("response"), Some(&expected));
}

#[test]
fn redact_self_redacting_callback_keeps_messages_and_response() {
    let details = details(
        Some(
            r#"{"id": "s", "messages": [{"role": "user"}], "response": {"choices": []}, "classifier_input": {}}"#,
        ),
        Some(r#"{"url": "http://x", "classifier_input": {}, "originating_request_masked": {}}"#),
    );
    let mut self_redacting = policy(true, &[]);
    self_redacting.callback_redacts_itself = true;
    let (object, request) = redacted(redact_standard_logging_payload(
        &details,
        &self_redacting,
        &stamp(),
    ));
    let messages: Json = serde_json::from_str(r#"[{"role": "user"}]"#).expect("the messages");
    assert_eq!(object.get("messages"), Some(&messages));
    let response: Json = serde_json::from_str(r#"{"choices": []}"#).expect("the response");
    assert_eq!(object.get("response"), Some(&response));
    assert!(!object.contains_key("classifier_input"));
    let cleaned = request.expect("the request is returned");
    assert!(!cleaned.contains_key("classifier_input"));
    assert!(!cleaned.contains_key("originating_request_masked"));
    assert!(cleaned.contains_key("url"));
}

#[rstest]
#[case::turn_off(true, false)]
#[case::turn_off_and_self_redacting(true, true)]
#[case::no_turn_off(false, false)]
fn redact_cleans_the_proxy_server_request_only_with_turn_off(
    #[case] turn_off: bool,
    #[case] redacts_itself: bool,
) {
    let details = details(
        Some(r#"{"id": "s"}"#),
        Some(r#"{"url": "http://x", "classifier_input": {}, "originating_request_masked": {}}"#),
    );
    let mut policy = policy(turn_off, &["unused"]);
    policy.callback_redacts_itself = redacts_itself;
    let (_, request) = redacted(redact_standard_logging_payload(&details, &policy, &stamp()));
    let returned = request.expect("the request is returned");
    assert_eq!(returned.contains_key("classifier_input"), !turn_off);
    assert_eq!(
        returned.contains_key("originating_request_masked"),
        !turn_off
    );
    assert!(returned.contains_key("url"));
}

fn payload_with_fields(fields: &[(&str, Json)]) -> StandardLoggingPayload {
    let mut raw: Json = serde_json::from_str(PAYLOAD).expect("a valid payload object");
    let Json::Object(map) = &mut raw else {
        panic!("the payload is an object");
    };
    for (field, value) in fields {
        map.insert((*field).to_owned(), value.clone());
    }
    let source = serde_json::to_string(&raw).expect("the payload encodes");
    serde_json::from_str(&source).expect("the payload parses")
}

fn with_messages(source: &str) -> StandardLoggingPayload {
    payload_with_fields(&[(
        "messages",
        serde_json::from_str(source).expect("the messages parse"),
    )])
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("a json value")
}

#[test]
fn base64_redacts_data_uris_in_content_and_other_keys() {
    let payload = with_messages(
        r#"[{"role": "user", "content": "data:image/png;base64,AAAA", "name": "data:audio/mp3;base64,BBBB"}]"#,
    );
    let stripped = strip_base64_from_messages(&payload, 10).expect("the pattern compiles");
    let expected =
        json(r#"[{"role": "user", "content": "[BASE64_REDACTED]", "name": "[BASE64_REDACTED]"}]"#);
    assert_eq!(stripped.messages, Some(expected));
}

#[test]
fn base64_matches_a_data_uri_across_whitespace() {
    let payload =
        with_messages(r#"[{"role": "user", "content": "data:image/png;base64,AAAA\nBBBB"}]"#);
    let stripped = strip_base64_from_messages(&payload, 10).expect("the pattern compiles");
    let expected = json(r#"[{"role": "user", "content": "[BASE64_REDACTED]"}]"#);
    assert_eq!(stripped.messages, Some(expected));
}

#[test]
fn base64_drops_file_and_non_text_content_items() {
    let payload = with_messages(
        r#"[{"role": "user", "content": [{"type": "text", "text": "ok"}, {"type": "image_url", "image_url": {}}, {"file": {}}, {"type": "file"}, {"no_type": true}, 5]}]"#,
    );
    let stripped = strip_base64_from_messages(&payload, 10).expect("the pattern compiles");
    let expected = json(
        r#"[{"role": "user", "content": [{"type": "text", "text": "ok"}, {"no_type": true}, 5]}]"#,
    );
    assert_eq!(stripped.messages, Some(expected));
}

#[test]
fn base64_drops_messages_that_are_not_objects() {
    let payload = with_messages(r#"["text", {"role": "user", "content": "hi"}]"#);
    let stripped = strip_base64_from_messages(&payload, 10).expect("the pattern compiles");
    assert_eq!(
        stripped.messages,
        Some(json(r#"[{"role": "user", "content": "hi"}]"#))
    );
}

#[test]
fn base64_adds_a_null_content_to_a_message_without_one() {
    let payload = with_messages(r#"[{"role": "system", "name": "n"}]"#);
    let stripped = strip_base64_from_messages(&payload, 10).expect("the pattern compiles");
    let expected = json(r#"[{"role": "system", "name": "n", "content": null}]"#);
    assert_eq!(stripped.messages, Some(expected));
}

#[test]
fn base64_leaves_the_response_untouched() {
    let response = json(r#"{"messages": "data:image/png;base64,AAAA"}"#);
    let payload = payload_with_fields(&[
        ("messages", json(r#"[{"role": "user", "content": "hi"}]"#)),
        ("response", response.clone()),
    ]);
    let stripped = strip_base64_from_messages(&payload, 10).expect("the pattern compiles");
    assert_eq!(stripped.response, Some(response));
}

#[test]
fn base64_walks_a_value_at_the_max_depth_and_stops_past_it() {
    let payload =
        with_messages(r#"[{"role": "user", "content": [[["data:image/png;base64,AAAA"]]]}]"#);
    let at_two = strip_base64_from_messages(&payload, 2).expect("the pattern compiles");
    let expected = json(r#"[{"role": "user", "content": [[["[BASE64_REDACTED]"]]]}]"#);
    assert_eq!(at_two.messages, Some(expected));
    let at_zero = strip_base64_from_messages(&payload, 0).expect("the pattern compiles");
    let expected_zero = json(r#"[{"role": "user", "content": [["[MAX_DEPTH_REACHED]"]]}]"#);
    assert_eq!(at_zero.messages, Some(expected_zero));
    assert_eq!(MAX_DEPTH_REACHED, "[MAX_DEPTH_REACHED]");
}

#[test]
fn base64_rejects_every_value_at_a_negative_depth() {
    let payload = with_messages(r#"[{"role": "user", "content": "hi"}]"#);
    let stripped = strip_base64_from_messages(&payload, -1).expect("the pattern compiles");
    let expected = json(r#"[{"role": "[MAX_DEPTH_REACHED]", "content": ["[MAX_DEPTH_REACHED]"]}]"#);
    assert_eq!(stripped.messages, Some(expected));
}

#[rstest]
#[case::missing(None)]
#[case::null(Some(Json::Null))]
#[case::text(Some(Json::Text("x".to_owned())))]
#[case::empty(Some(Json::List(Vec::new())))]
fn base64_leaves_messages_that_are_not_a_non_empty_list_alone(#[case] messages: Option<Json>) {
    let payload = messages.map_or_else(payload, |value| payload_with("messages", value));
    let stripped = strip_base64_from_messages(&payload, 10).expect("the pattern compiles");
    assert_eq!(stripped.messages.as_ref(), payload.messages.as_ref());
}

#[test]
fn base64_returns_a_new_payload() {
    let payload = with_messages(r#"[{"role": "user", "content": "data:image/png;base64,AAAA"}]"#);
    let before = encoded(&payload);
    let stripped = strip_base64_from_messages(&payload, 10).expect("the pattern compiles");
    assert_eq!(encoded(&payload), before);
    assert_ne!(encoded(&stripped), before);
}

#[derive(Default)]
struct RecordingCounter {
    calls: Mutex<Vec<String>>,
}

impl RecordingCounter {
    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("the counter lock is not poisoned")
            .clone()
    }
}

impl CallbackFailureCounter for RecordingCounter {
    fn increment_callback_logging_failure(&self, callback_name: &str) {
        self.calls
            .lock()
            .expect("the counter lock is not poisoned")
            .push(callback_name.to_owned());
    }
}

#[test]
fn failure_counter_increments_only_the_first_counter() {
    let first = RecordingCounter::default();
    let second = RecordingCounter::default();
    let counters: [&dyn CallbackFailureCounter; 2] = [&first, &second];
    handle_callback_failure("datadog", &counters);
    assert_eq!(first.calls(), vec!["datadog"]);
    assert!(second.calls().is_empty());
}

#[test]
fn failure_counter_stops_at_the_first_counter_in_order() {
    let first = RecordingCounter::default();
    let middle = RecordingCounter::default();
    let last = RecordingCounter::default();
    let counters: [&dyn CallbackFailureCounter; 3] = [&first, &middle, &last];
    handle_callback_failure("newrelic", &counters);
    assert_eq!(first.calls(), vec!["newrelic"]);
    assert!(middle.calls().is_empty());
    assert!(last.calls().is_empty());
}

#[test]
fn failure_counter_does_nothing_without_a_counter() {
    handle_callback_failure("datadog", &[]);
}

#[test]
fn health_healthy_serializes_with_a_null_message() {
    let status = IntegrationHealthStatus::healthy();
    assert_eq!(status.status, HealthState::Healthy);
    let encoded = serde_json::to_string(&status).expect("the status serializes");
    let actual: Json = serde_json::from_str(&encoded).expect("the status is json");
    assert_eq!(
        actual,
        json(r#"{"status": "healthy", "error_message": null}"#)
    );
}

#[test]
fn health_unhealthy_serializes_with_its_message() {
    let status = IntegrationHealthStatus::unhealthy("boom".to_owned());
    assert_eq!(status.status, HealthState::Unhealthy);
    assert_eq!(status.error_message.as_deref(), Some("boom"));
    let encoded = serde_json::to_string(&status).expect("the status serializes");
    let actual: Json = serde_json::from_str(&encoded).expect("the status is json");
    assert_eq!(
        actual,
        json(r#"{"status": "unhealthy", "error_message": "boom"}"#)
    );
    assert_eq!(
        serde_json::to_string(&HealthState::Unhealthy).expect("the state serializes"),
        "\"unhealthy\""
    );
}

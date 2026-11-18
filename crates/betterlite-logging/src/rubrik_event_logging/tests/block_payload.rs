use betterlite_logging::rubrik_event_logging::block_payload::prepare_block_failure_payload;
use betterlite_logging::rubrik_event_logging::error::EventPayloadError;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;
use tracing::Level;
use uuid::Uuid;

use super::support::{Harness, details, exception, field, json, object_keys, text};

const BASE: &str = r#"{"litellm_call_id":"call-1","standard_logging_object":{"id":"chatcmpl-provider","model":"gpt-4o","status":"success","response":"provider text","messages":[{"role":"user","content":"hi"}],"litellm_params":{"custom":true}}}"#;

const FALLBACK: &str = r#"{"litellm_call_id":"call-1","model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"start_time":"2026-01-01T00:00:00.5Z"}"#;

fn payload(call_details: &str) -> Json {
    payload_with_metadata(call_details, "{}")
}

fn payload_with_metadata(call_details: &str, metadata: &str) -> Json {
    let source = details(call_details);
    let caller_metadata = details(metadata);
    let raised = exception("blocked by policy", "rubrik");
    Json::Object(
        prepare_block_failure_payload(&source, &raised, &caller_metadata)
            .expect("the payload is prepared"),
    )
}

fn payload_result(call_details: &str) -> Result<Json, EventPayloadError> {
    let source = details(call_details);
    let caller_metadata = details("{}");
    let raised = exception("blocked by policy", "rubrik");
    prepare_block_failure_payload(&source, &raised, &caller_metadata).map(Json::Object)
}

fn assert_generated_id(id: &str) {
    let suffix = id
        .strip_prefix("chatcmpl-")
        .expect("the generated id carries the prefix");
    let parsed = Uuid::parse_str(suffix).expect("the suffix is a UUID");
    assert_eq!(parsed.get_version_num(), 4);
}

#[test]
fn a_copied_base_keeps_its_fields() {
    let payload = payload(BASE);
    assert_eq!(text(field(&payload, "model")), "gpt-4o");
    assert_eq!(text(field(&payload, "status")), "success");
    assert_eq!(
        field(&payload, "litellm_params"),
        &json(r#"{"custom":true}"#)
    );
}

#[test]
fn the_copied_base_response_is_the_exception_text() {
    let payload = payload(BASE);
    assert_eq!(
        text(field(&payload, "response")),
        "ModifyResponseException: blocked by policy"
    );
}

#[test]
fn the_copied_base_id_is_the_call_id() {
    let payload = payload(BASE);
    assert_eq!(text(field(&payload, "id")), "call-1");
}

#[test]
fn the_copied_base_is_a_deep_copy() {
    let source = details(BASE);
    let caller_metadata = details("{}");
    let raised = exception("blocked by policy", "rubrik");
    let mut payload = prepare_block_failure_payload(&source, &raised, &caller_metadata)
        .expect("the payload is prepared");
    let Json::Object(params) = payload
        .get_mut("litellm_params")
        .expect("the params are copied")
    else {
        panic!("the litellm params are an object");
    };
    params.insert("custom".to_owned(), Json::Bool(false));
    let standard = json(BASE);
    assert_eq!(
        field(
            field(&standard, "standard_logging_object"),
            "litellm_params"
        ),
        &json(r#"{"custom":true}"#)
    );
}

#[test]
fn a_copied_base_without_a_call_id_gets_a_generated_id() {
    let payload = payload(r#"{"standard_logging_object":{"id":"chatcmpl-provider"}}"#);
    let id = text(field(&payload, "id"));
    assert_ne!(id, "chatcmpl-provider");
    assert_generated_id(id);
}

#[test]
fn a_copied_base_with_an_empty_call_id_gets_a_generated_id() {
    let payload =
        payload(r#"{"litellm_call_id":"","standard_logging_object":{"id":"chatcmpl-provider"}}"#);
    assert_generated_id(text(field(&payload, "id")));
}

#[rstest]
#[case::absent(r#"{"litellm_call_id":"call-1","model":"gpt-4o"}"#)]
#[case::null(r#"{"litellm_call_id":"call-1","model":"gpt-4o","standard_logging_object":null}"#)]
fn an_unset_standard_logging_object_uses_the_fallback(#[case] call_details: &str) {
    let payload = payload(call_details);
    assert_eq!(text(field(&payload, "status")), "failure");
    assert_eq!(text(field(&payload, "model")), "gpt-4o");
}

#[test]
fn a_missing_standard_logging_object_is_logged_before_the_fallback() {
    let harness = Harness::new(&[], &[]);
    let log_payload = payload(FALLBACK);
    assert_eq!(text(field(&log_payload, "status")), "failure");
    let debug = harness.logs.with_level(Level::DEBUG);
    assert_eq!(debug.len(), 1);
    assert_eq!(
        debug[0].message,
        "Rubrik: standard_logging_object not yet on model_call_details; using the fallback payload"
    );
    assert_eq!(debug[0].litellm_call_id.as_deref(), Some("call-1"));
}

#[test]
fn a_standard_logging_object_that_is_not_an_object_is_an_error() {
    let error = payload_result(r#"{"litellm_call_id":"call-1","standard_logging_object":"text"}"#)
        .expect_err("a wrong shape is an error");
    assert!(matches!(
        error,
        EventPayloadError::FieldType {
            field: "standard_logging_object"
        }
    ));
}

#[test]
fn the_fallback_model_group_is_the_model() {
    let payload = payload(FALLBACK);
    assert_eq!(text(field(&payload, "model_group")), "gpt-4o");
}

#[test]
fn the_fallback_model_id_is_empty() {
    let payload = payload(FALLBACK);
    assert_eq!(text(field(&payload, "model_id")), "");
}

#[rstest]
#[case::absent(r#"{"litellm_call_id":"call-1","start_time":"2026-01-01T00:00:00Z"}"#)]
#[case::null(r#"{"litellm_call_id":"call-1","model":null,"start_time":"2026-01-01T00:00:00Z"}"#)]
fn the_fallback_without_a_model_uses_empty_names(#[case] call_details: &str) {
    let payload = payload(call_details);
    assert_eq!(text(field(&payload, "model")), "");
    assert_eq!(text(field(&payload, "model_group")), "");
}

#[test]
fn the_fallback_status_is_failure() {
    let payload = payload(FALLBACK);
    assert_eq!(text(field(&payload, "status")), "failure");
}

#[test]
fn the_fallback_response_is_the_exception_text() {
    let payload = payload(FALLBACK);
    assert_eq!(
        text(field(&payload, "response")),
        "ModifyResponseException: blocked by policy"
    );
}

#[test]
fn the_fallback_keeps_only_logging_parameters() {
    let payload = payload(
        r#"{"litellm_call_id":"call-1","optional_params":{"temperature":0.5,"max_tokens":5,"stream":true,"metadata":{"a":1},"system":"x","unknown_parameter":1}}"#,
    );
    assert_eq!(
        object_keys(field(&payload, "model_parameters")),
        ["temperature", "max_tokens", "stream"]
    );
    assert_eq!(
        field(&payload, "model_parameters"),
        &json(r#"{"temperature":0.5,"max_tokens":5,"stream":true}"#)
    );
}

#[rstest]
#[case::absent(r#"{"litellm_call_id":"call-1","model":"gpt-4o"}"#)]
#[case::null(r#"{"litellm_call_id":"call-1","model":"gpt-4o","optional_params":null}"#)]
fn the_fallback_parameters_are_empty_without_optional_params(#[case] call_details: &str) {
    let payload = payload(call_details);
    assert_eq!(field(&payload, "model_parameters"), &json("{}"));
}

#[test]
fn a_wrong_typed_optional_params_is_an_error() {
    let error = payload_result(r#"{"litellm_call_id":"call-1","optional_params":"text"}"#)
        .expect_err("a wrong shape is an error");
    assert!(matches!(
        error,
        EventPayloadError::FieldType {
            field: "optional_params"
        }
    ));
}

#[test]
fn the_fallback_times_are_the_start_time_as_unix_seconds() {
    let payload = payload(FALLBACK);
    let expected = Json::Number(Number::from_f64(1_767_225_600.5).expect("a finite float"));
    assert_eq!(field(&payload, "startTime"), &expected);
    assert_eq!(field(&payload, "endTime"), &expected);
    assert_eq!(field(&payload, "completionStartTime"), &expected);
}

#[test]
fn an_offset_start_time_is_converted_to_utc() {
    let payload =
        payload(r#"{"litellm_call_id":"call-1","start_time":"2026-01-01T02:00:00+02:00"}"#);
    let expected = Json::Number(Number::from_f64(1_767_225_600.0).expect("a finite float"));
    assert_eq!(field(&payload, "startTime"), &expected);
}

#[rstest]
#[case::absent(r#"{"litellm_call_id":"call-1","model":"gpt-4o"}"#)]
#[case::null(r#"{"litellm_call_id":"call-1","model":"gpt-4o","start_time":null}"#)]
fn the_fallback_times_are_null_without_a_start_time(#[case] call_details: &str) {
    let payload = payload(call_details);
    assert_eq!(field(&payload, "startTime"), &Json::Null);
    assert_eq!(field(&payload, "endTime"), &Json::Null);
    assert_eq!(field(&payload, "completionStartTime"), &Json::Null);
}

#[test]
fn an_unparseable_start_time_is_an_error() {
    let error = payload_result(r#"{"litellm_call_id":"call-1","start_time":"not a time"}"#)
        .expect_err("an unparseable time is an error");
    assert!(matches!(error, EventPayloadError::StartTime { .. }));
}

#[test]
fn a_wrong_typed_start_time_is_an_error() {
    let error = payload_result(r#"{"litellm_call_id":"call-1","start_time":5}"#)
        .expect_err("a numeric time is an error");
    assert!(matches!(error, EventPayloadError::CallDetails { .. }));
}

#[test]
fn the_fallback_messages_come_from_the_call_details() {
    let payload = payload(FALLBACK);
    assert_eq!(
        field(&payload, "messages"),
        &json(r#"[{"role":"user","content":"hi"}]"#)
    );
}

#[rstest]
#[case::absent(r#"{"litellm_call_id":"call-1","model":"gpt-4o"}"#)]
#[case::null(r#"{"litellm_call_id":"call-1","model":"gpt-4o","messages":null}"#)]
fn the_fallback_messages_are_empty_without_messages(#[case] call_details: &str) {
    let payload = payload(call_details);
    assert_eq!(field(&payload, "messages"), &json("[]"));
}

#[test]
fn the_fallback_metadata_is_the_caller_metadata() {
    let payload = payload_with_metadata(
        FALLBACK,
        r#"{"user_api_key_user_id":"u-1","user_api_key_alias":"alias"}"#,
    );
    assert_eq!(
        object_keys(field(&payload, "metadata")),
        ["user_api_key_user_id", "user_api_key_alias"]
    );
    assert_eq!(
        field(&payload, "metadata"),
        &json(r#"{"user_api_key_user_id":"u-1","user_api_key_alias":"alias"}"#)
    );
}

#[test]
fn the_fallback_id_is_the_call_id() {
    let payload = payload(FALLBACK);
    assert_eq!(text(field(&payload, "id")), "call-1");
}

#[test]
fn a_fallback_without_a_call_id_gets_a_generated_id() {
    let payload = payload(r#"{"model":"gpt-4o"}"#);
    assert_generated_id(text(field(&payload, "id")));
}

#[test]
fn the_system_prompt_is_prepended_to_the_copied_base() {
    let payload = payload(
        r#"{"litellm_call_id":"call-1","system":"be nice","standard_logging_object":{"messages":[{"role":"user","content":"hi"}]}}"#,
    );
    assert_eq!(
        field(&payload, "messages"),
        &json(r#"[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]"#)
    );
}

#[test]
fn the_system_prompt_is_prepended_to_the_fallback() {
    let payload = payload(
        r#"{"litellm_call_id":"call-1","system":"be nice","messages":[{"role":"user","content":"hi"}]}"#,
    );
    assert_eq!(
        field(&payload, "messages"),
        &json(r#"[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]"#)
    );
}

#[test]
fn a_system_prompt_without_base_messages_adds_no_messages() {
    let payload = payload(
        r#"{"litellm_call_id":"call-1","system":"be nice","standard_logging_object":{"id":"chatcmpl-provider"}}"#,
    );
    let Json::Object(fields) = payload else {
        panic!("the payload is an object");
    };
    assert!(!fields.contains_key("messages"));
}

#[test]
fn a_system_prompt_without_fallback_messages_keeps_the_system_prompt() {
    let payload = payload(r#"{"litellm_call_id":"call-1","system":"be nice"}"#);
    assert_eq!(
        field(&payload, "messages"),
        &json(r#"[{"role":"system","content":"be nice"}]"#)
    );
}

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rstest::rstest;

use crate::datadog::apm::NullTracer;
use crate::datadog::payloads::legacy::{LegacyLoggingKwargs, legacy_v1_logging_to_datadog};
use crate::datadog::payloads::status::DataDogStatus;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, MAX_DEPTH_MARKER};

use super::support::{
    default_environment, default_kwargs, field, json, legacy_event, message, nested_json,
};

const KWARGS: &str = r#"{
    "model": "gpt-4o",
    "user": "user-1",
    "messages": [{"role": "user", "content": "hi"}],
    "litellm_params": {"metadata": {"endpoint": "x", "caching_groups": ["a"], "previous_models": ["b"], "team_id": "team-1"}},
    "optional_params": {"temperature": 0.2},
    "call_type": "completion",
    "cache_hit": true,
    "response_cost": 0.0015
}"#;

const RESPONSE: &str =
    r#"{"id": "chatcmpl-1", "usage": {"prompt_tokens": 2, "completion_tokens": 3}, "choices": []}"#;

fn kwargs() -> LegacyLoggingKwargs {
    serde_json::from_str(KWARGS).unwrap()
}

fn legacy_message(kwargs: &LegacyLoggingKwargs, response: &Json) -> Json {
    let built = legacy_v1_logging_to_datadog(
        &legacy_event(kwargs, response),
        &default_environment(),
        &NullTracer,
    )
    .unwrap();
    message(&built)
}

#[test]
fn datadog_legacy_v1_logging_behavior() {
    let kwargs = kwargs();
    let response = json(RESPONSE);
    let built = legacy_v1_logging_to_datadog(
        &legacy_event(&kwargs, &response),
        &default_environment(),
        &NullTracer,
    )
    .unwrap();
    assert_eq!(built.status, DataDogStatus::Info);
    let tags = "env:unknown,service:litellm-server,version:unknown,HOSTNAME:,POD_NAME:unknown";
    assert_eq!(built.ddtags, tags);
    let expected = json(
        r#"{
        "id": "chatcmpl-1",
        "call_type": "completion",
        "cache_hit": true,
        "start_time": "2024-01-01T00:00:00.000000+00:00",
        "end_time": "2024-01-01T00:00:01.500000+00:00",
        "response_time": 1500.0,
        "model": "gpt-4o",
        "user": "user-1",
        "model_parameters": {"temperature": 0.2},
        "spend": 0.0015,
        "messages": [{"role": "user", "content": "hi"}],
        "response": {"id": "chatcmpl-1", "usage": {"prompt_tokens": 2, "completion_tokens": 3}, "choices": []},
        "usage": {"prompt_tokens": 2, "completion_tokens": 3},
        "metadata": {"team_id": "team-1"}
    }"#,
    );
    assert_eq!(message(&built), expected);
}

fn default_record() -> Json {
    json(
        r#"{
        "id": "generated-id",
        "call_type": "litellm.completion",
        "cache_hit": false,
        "start_time": "2024-01-01T00:00:00.000000+00:00",
        "end_time": "2024-01-01T00:00:01.500000+00:00",
        "response_time": 1500.0,
        "model": "",
        "user": "",
        "model_parameters": {},
        "spend": 0,
        "messages": null,
        "response": {"usage": {}},
        "usage": {},
        "metadata": {}
    }"#,
    )
}

#[test]
fn datadog_legacy_v1_logging_defaults() {
    let kwargs = default_kwargs();
    let response = json(r#"{"usage": {}}"#);
    assert_eq!(legacy_message(&kwargs, &response), default_record());
}

#[test]
fn datadog_legacy_v1_logging_resolves_explicit_nulls_to_defaults() {
    let kwargs: LegacyLoggingKwargs = serde_json::from_str(
        r#"{"model": null, "user": null, "messages": null, "litellm_params": null, "optional_params": null, "call_type": null, "cache_hit": null, "response_cost": null}"#,
    )
    .unwrap();
    let response = json(r#"{"usage": {}}"#);
    assert_eq!(legacy_message(&kwargs, &response), default_record());
}

#[test]
fn datadog_legacy_v1_logging_generates_the_id_for_a_null_id() {
    let kwargs = default_kwargs();
    let body = legacy_message(&kwargs, &json(r#"{"id": null, "usage": {}}"#));
    assert_eq!(field(&body, "id"), &json(r#""generated-id""#));
}

#[test]
fn datadog_legacy_v1_logging_writes_a_null_response_time_that_overflows() {
    let kwargs = default_kwargs();
    let response = json(r#"{"usage": {}}"#);
    let mut event = legacy_event(&kwargs, &response);
    event.start_time = DateTime::<Utc>::MIN_UTC.fixed_offset();
    event.end_time = DateTime::<Utc>::MAX_UTC.fixed_offset();
    let built = legacy_v1_logging_to_datadog(&event, &default_environment(), &NullTracer).unwrap();
    assert_eq!(field(&message(&built), "response_time"), &Json::Null);
}

#[test]
fn datadog_legacy_v1_logging_uses_the_injected_status() {
    let kwargs = kwargs();
    let response = json(RESPONSE);
    let mut event = legacy_event(&kwargs, &response);
    event.status = DataDogStatus::Error;
    let built = legacy_v1_logging_to_datadog(&event, &default_environment(), &NullTracer).unwrap();
    assert_eq!(built.status, DataDogStatus::Error);
}

#[rstest]
#[case::endpoint(
    r#"{"endpoint": "x", "team_id": "team-1"}"#,
    r#"{"team_id": "team-1"}"#
)]
#[case::caching_groups(r#"{"caching_groups": [], "user": "u"}"#, r#"{"user": "u"}"#)]
#[case::previous_models(r#"{"previous_models": [1], "model": "m"}"#, r#"{"model": "m"}"#)]
fn datadog_legacy_v1_logging_cleans_metadata(#[case] metadata: &str, #[case] expected: &str) {
    let source = format!(r#"{{"litellm_params": {{"metadata": {metadata}}}}}"#);
    let kwargs: LegacyLoggingKwargs = serde_json::from_str(&source).unwrap();
    let response = json(r#"{"usage": {}}"#);
    let body = legacy_message(&kwargs, &response);
    assert_eq!(field(&body, "metadata"), &json(expected));
}

#[test]
fn datadog_legacy_v1_logging_strips_nul_bytes() {
    let mut messages = IndexMap::new();
    messages.insert("a\0b".to_owned(), Json::Text("c\0d".to_owned()));
    let mut kwargs = default_kwargs();
    kwargs.model = "m\0odel".to_owned();
    kwargs.messages = Json::Object(messages);
    kwargs
        .litellm_params
        .metadata
        .insert("k\0ey".to_owned(), Json::Text("v\0al".to_owned()));
    let body = legacy_message(&kwargs, &json(r#"{"usage": {}}"#));
    assert_eq!(field(&body, "model"), &json(r#""model""#));
    assert_eq!(field(&body, "messages"), &json(r#"{"ab": "cd"}"#));
    assert_eq!(field(&body, "metadata"), &json(r#"{"key": "val"}"#));
}

#[test]
fn datadog_legacy_v1_logging_marks_values_past_the_recursion_limit() {
    let mut kwargs = default_kwargs();
    kwargs.messages = nested_json(DEFAULT_MAX_RECURSE_DEPTH + 1);
    let response = json(r#"{"usage": {}}"#);
    let built = legacy_v1_logging_to_datadog(
        &legacy_event(&kwargs, &response),
        &default_environment(),
        &NullTracer,
    )
    .unwrap();
    assert!(built.message.contains(MAX_DEPTH_MARKER));
}

#[rstest]
#[case::not_an_object(r#""text""#, "the legacy response object is not a JSON object")]
#[case::missing_usage(r#"{"id": "x"}"#, "the legacy response object has no usage")]
#[case::usage_not_an_object(
    r#"{"usage": 5}"#,
    "the legacy response object's usage is not a JSON object"
)]
#[case::id_not_a_string(
    r#"{"id": 5, "usage": {}}"#,
    "the legacy response object's id is not a string"
)]
fn datadog_legacy_v1_logging_rejects_bad_responses(#[case] encoded: &str, #[case] expected: &str) {
    let kwargs = default_kwargs();
    let response = json(encoded);
    let error = legacy_v1_logging_to_datadog(
        &legacy_event(&kwargs, &response),
        &default_environment(),
        &NullTracer,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), expected);
}

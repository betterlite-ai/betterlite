use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{expected, row, row_json};

const COLUMNS: [&str; 41] = [
    "id",
    "trace_id",
    "session_id",
    "litellm_call_id",
    "call_type",
    "status",
    "model",
    "model_group",
    "model_id",
    "custom_llm_provider",
    "api_base",
    "stream",
    "cache_hit",
    "start_time",
    "end_time",
    "completion_start_time",
    "response_time",
    "prompt_tokens",
    "completion_tokens",
    "total_tokens",
    "response_cost",
    "saved_cache_cost",
    "api_key_hash",
    "api_key_alias",
    "team_id",
    "team_alias",
    "user_id",
    "org_id",
    "end_user",
    "requester_ip_address",
    "user_agent",
    "request_tags",
    "messages",
    "response",
    "error_str",
    "error_information",
    "metadata",
    "model_parameters",
    "hidden_params",
    "guardrail_information",
    "cost_breakdown",
];

#[test]
fn an_empty_payload_has_every_column_null_in_the_documented_order() {
    let json = row_json(&row("{}"));
    let columns: Vec<&str> = json.keys().map(String::as_str).collect();
    assert_eq!(columns, COLUMNS);
    assert!(json.values().all(|value| matches!(value, Json::Null)));
}

#[rstest]
#[case::prompt_tokens("prompt_tokens")]
#[case::completion_tokens("completion_tokens")]
#[case::total_tokens("total_tokens")]
#[case::response_time("response_time")]
#[case::response_cost("response_cost")]
#[case::saved_cache_cost("saved_cache_cost")]
fn a_boolean_is_rejected_in_a_numeric_column(#[case] column: &str) {
    let json = row_json(&row(&format!(r#"{{"{column}": true}}"#)));
    assert_eq!(json[column], expected("null"));
}

#[rstest]
#[case::text("id", r#""5""#)]
#[case::boolean("error_str", "true")]
#[case::object("model", r#"{"name": "gpt-4o"}"#)]
fn a_wrongly_typed_text_field_is_null(#[case] column: &str, #[case] value: &str) {
    let json = row_json(&row(&format!(r#"{{"{column}": {value}}}"#)));
    assert_eq!(json[column], expected("null"));
}

#[test]
fn booleans_are_accepted_as_stream_and_cache_hit() {
    let row = row(r#"{"stream": true, "cache_hit": false}"#);
    assert_eq!(row.stream, Some(true));
    assert_eq!(row.cache_hit, Some(false));
}

#[rstest]
#[case::text(r#""yes""#)]
#[case::number("1")]
#[case::null("null")]
fn a_non_boolean_stream_is_null(#[case] value: &str) {
    let row = row(&format!(r#"{{"stream": {value}, "cache_hit": {value}}}"#));
    assert_eq!(row.stream, None);
    assert_eq!(row.cache_hit, None);
}

#[test]
fn a_missing_stream_or_cache_hit_is_null() {
    let row = row("{}");
    assert_eq!(row.stream, None);
    assert_eq!(row.cache_hit, None);
}

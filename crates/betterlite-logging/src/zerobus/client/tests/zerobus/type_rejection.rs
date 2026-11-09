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
#[case::number("id", "5")]
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

#[test]
fn the_payload_scalars_land_in_their_columns() {
    let row = row(r#"{
            "id": "chatcmpl-1",
            "trace_id": "trace-1",
            "session_id": "session-1",
            "litellm_call_id": "call-1",
            "call_type": "acompletion",
            "status": "success",
            "model": "gpt-4o",
            "model_group": "gpt-4o-group",
            "model_id": "model-1",
            "custom_llm_provider": "openai",
            "api_base": "https://api.openai.com",
            "end_user": "end-user-1",
            "requester_ip_address": "10.0.0.1",
            "user_agent": "curl/8"
        }"#);
    assert_eq!(row.id.as_deref(), Some("chatcmpl-1"));
    assert_eq!(row.trace_id.as_deref(), Some("trace-1"));
    assert_eq!(row.session_id.as_deref(), Some("session-1"));
    assert_eq!(row.litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(row.call_type.as_deref(), Some("acompletion"));
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.model.as_deref(), Some("gpt-4o"));
    assert_eq!(row.model_group.as_deref(), Some("gpt-4o-group"));
    assert_eq!(row.model_id.as_deref(), Some("model-1"));
    assert_eq!(row.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(row.api_base.as_deref(), Some("https://api.openai.com"));
    assert_eq!(row.end_user.as_deref(), Some("end-user-1"));
    assert_eq!(row.requester_ip_address.as_deref(), Some("10.0.0.1"));
    assert_eq!(row.user_agent.as_deref(), Some("curl/8"));
}

#[test]
fn response_time_lands_as_a_json_number() {
    let json = row_json(&row(r#"{"response_time": 1.25}"#));
    assert_eq!(json["response_time"], expected("1.25"));
}

#[test]
fn the_token_counts_land_in_their_columns() {
    let row = row(r#"{"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}"#);
    assert_eq!(row.prompt_tokens, Some(10));
    assert_eq!(row.completion_tokens, Some(5));
    assert_eq!(row.total_tokens, Some(15));
}

#[test]
fn a_fractional_count_is_truncated_toward_zero() {
    let row = row(r#"{"prompt_tokens": 10.9, "completion_tokens": -0.9}"#);
    assert_eq!(row.prompt_tokens, Some(10));
    assert_eq!(row.completion_tokens, Some(0));
}

#[test]
fn a_count_past_the_i64_range_is_null() {
    let row = row(r#"{"prompt_tokens": 1e25}"#);
    assert_eq!(row.prompt_tokens, None);
}

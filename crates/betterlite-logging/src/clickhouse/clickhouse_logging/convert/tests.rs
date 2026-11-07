#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{
    cache_tokens, find_traceparent, int_value, json_value, parse_traceparent, request_tags,
    session_id, strip_cache_hit_suffix, to_ms,
};
use crate::clickhouse::clickhouse_logging::tests::support::kwargs;

const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const SPAN_ID: &str = "00f067aa0be902b7";

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture json decodes")
}

fn usage_map(text: &str) -> IndexMap<String, Json> {
    match json(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the usage fixture is not an object")
        }
    }
}

#[rstest]
#[case::canonical("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0be902b7-01")]
#[case::uppercase("00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0BE902B7-01")]
#[case::surrounding_space(" 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0be902b7-01\n")]
#[case::future_version("ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0be902b7-00")]
fn parse_traceparent_accepts_w3c_headers(#[case] header: &str) {
    assert_eq!(
        parse_traceparent(header),
        (TRACE_ID.to_owned(), SPAN_ID.to_owned())
    );
}

#[rstest]
#[case::empty("")]
#[case::missing_flags("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0be902b7")]
#[case::extra_segment("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0be902b7-01-02")]
#[case::short_trace_id("00-4bf92f3577b34da6a3ce929d0e0e473-00f067aa0be902b7-01")]
#[case::long_span_id("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0be902b70-01")]
#[case::non_hex("00-4bf92f3577b34da6a3ce929d0e0e473g-00f067aa0be902b7-01")]
#[case::zero_trace_id("00-00000000000000000000000000000000-00f067aa0be902b7-01")]
#[case::zero_span_id("00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01")]
fn parse_traceparent_rejects_invalid_headers(#[case] header: &str) {
    assert_eq!(parse_traceparent(header), (String::new(), String::new()));
}

#[rstest]
#[case::timestamp("chatcmpl-1_cache_hit1712345678.123", "chatcmpl-1")]
#[case::no_digits("chatcmpl-1_cache_hit", "chatcmpl-1")]
#[case::only_suffix("_cache_hit12", "")]
#[case::no_suffix("chatcmpl-1", "chatcmpl-1")]
#[case::letters_after_suffix("chatcmpl-1_cache_hitx", "chatcmpl-1_cache_hitx")]
#[case::several_dots("chatcmpl-1_cache_hit1.2.3", "chatcmpl-1")]
#[case::last_suffix_wins("a_cache_hit1_cache_hit2", "a_cache_hit1")]
fn strip_cache_hit_suffix_removes_the_terminal_suffix(#[case] request_id: &str, #[case] raw: &str) {
    assert_eq!(strip_cache_hit_suffix(request_id), raw);
}

#[rstest]
#[case::whole_seconds("2", Some(2000))]
#[case::fraction("0.123", Some(122))]
#[case::negative("-1.5", Some(-1500))]
#[case::zero("0", Some(0))]
#[case::text("\"2\"", None)]
#[case::flag("true", None)]
#[case::null("null", None)]
fn to_ms_multiplies_seconds_into_truncated_milliseconds(
    #[case] seconds: &str,
    #[case] expected: Option<i64>,
) {
    let decoded = json(seconds);
    assert_eq!(to_ms(Some(&decoded)), expected);
}

#[test]
fn to_ms_returns_none_when_the_time_is_absent() {
    assert_eq!(to_ms(None), None);
}

#[rstest]
#[case::integer("3", 3)]
#[case::negative("-2", -2)]
#[case::float("3.0", 0)]
#[case::flag("true", 0)]
#[case::text("\"3\"", 0)]
#[case::null("null", 0)]
fn int_value_only_accepts_json_integers(#[case] input: &str, #[case] expected: i64) {
    let decoded = json(input);
    assert_eq!(int_value(Some(&decoded)), expected);
}

#[test]
fn int_value_returns_zero_when_the_value_is_absent() {
    assert_eq!(int_value(None), 0);
}

#[rstest]
#[case::absent(None, "")]
#[case::null(Some("null"), "")]
#[case::empty_text(Some("\"\""), "")]
#[case::text(Some("\"hello\""), "hello")]
#[case::number(Some("3"), "3")]
#[case::list(Some("[1,2]"), "[1,2]")]
#[case::object(Some("{\"a\":1}"), "{\"a\":1}")]
fn json_value_keeps_text_and_serializes_everything_else(
    #[case] input: Option<&str>,
    #[case] expected: &str,
) {
    let owned = input.map(json);
    assert_eq!(json_value(owned.as_ref()), expected);
}

#[test]
fn cache_tokens_reads_openai_prompt_token_details() {
    let tokens = cache_tokens(&usage_map(
        r#"{"prompt_tokens_details": {"cached_tokens": 7, "cache_write_tokens": 3, "cache_creation_tokens": 9}}"#,
    ));
    assert_eq!(tokens, (7, 3));
}

#[test]
fn cache_tokens_falls_back_to_anthropic_cache_fields() {
    let tokens = cache_tokens(&usage_map(
        r#"{"cache_read_input_tokens": 11, "cache_creation_input_tokens": 5}"#,
    ));
    assert_eq!(tokens, (11, 5));
}

#[test]
fn cache_tokens_reads_the_anthropic_cache_creation_field() {
    let tokens = cache_tokens(&usage_map(
        r#"{"prompt_tokens_details": {"cache_creation_tokens": 4}}"#,
    ));
    assert_eq!(tokens, (0, 4));
}

#[test]
fn cache_tokens_ignores_a_non_object_prompt_token_details() {
    let tokens = cache_tokens(&usage_map(
        r#"{"prompt_tokens_details": "cached", "cache_read_input_tokens": 2}"#,
    ));
    assert_eq!(tokens, (2, 0));
}

#[test]
fn find_traceparent_searches_custom_headers_first() {
    let metadata = usage_map(&format!(
        r#"{{"requester_custom_headers": {{"TraceParent": "00-{TRACE_ID}-{SPAN_ID}-01"}}}}"#
    ));
    let event = kwargs(&format!(
        r#"{{"litellm_params": {{"proxy_server_request": {{"headers": {{"traceparent": "00-{TRACE_ID}-0000000000000000-01"}}}}}}}}"#
    ));
    assert_eq!(
        find_traceparent(Some(&metadata), &event),
        (TRACE_ID.to_owned(), SPAN_ID.to_owned())
    );
}

#[test]
fn find_traceparent_falls_back_to_proxy_request_headers() {
    let event = kwargs(&format!(
        r#"{{"litellm_params": {{"proxy_server_request": {{"headers": {{"TraceParent": "00-{TRACE_ID}-{SPAN_ID}-01"}}}}}}}}"#
    ));
    let metadata = usage_map("{}");
    assert_eq!(
        find_traceparent(Some(&metadata), &event),
        (TRACE_ID.to_owned(), SPAN_ID.to_owned())
    );
}

#[test]
fn find_traceparent_stops_when_the_custom_header_is_invalid() {
    let metadata =
        usage_map(r#"{"requester_custom_headers": {"traceparent": "not-a-traceparent"}}"#);
    let event = kwargs(&format!(
        r#"{{"litellm_params": {{"proxy_server_request": {{"headers": {{"traceparent": "00-{TRACE_ID}-{SPAN_ID}-01"}}}}}}}}"#
    ));
    assert_eq!(
        find_traceparent(Some(&metadata), &event),
        (String::new(), String::new())
    );
}

#[test]
fn find_traceparent_returns_empty_when_no_header_exists() {
    assert_eq!(
        find_traceparent(None, &kwargs("{}")),
        (String::new(), String::new())
    );
}

#[rstest]
#[case::strings(r#"["team-a","team-b"]"#, &["team-a", "team-b"])]
#[case::mixed(r#"["team-a",2,true,null]"#, &["team-a", "2", "True", "None"])]
#[case::not_a_list("\"team-a\"", &[])]
#[case::null("null", &[])]
fn request_tags_stringifies_every_item(#[case] input: &str, #[case] expected: &[&str]) {
    let decoded = json(input);
    let tags = request_tags(Some(&decoded));
    let wanted: Vec<String> = expected.iter().map(|tag| (*tag).to_owned()).collect();
    assert_eq!(tags, wanted);
}

#[test]
fn session_id_prefers_the_payload_session_id() {
    let event = kwargs(
        r#"{"standard_logging_object": {"session_id": "payload", "trace_id": "trace"},
            "litellm_params": {"metadata": {"session_id": "metadata"}}}"#,
    );
    let payload = event
        .standard_logging_object
        .as_ref()
        .expect("the fixture carries a payload");
    assert_eq!(session_id(payload, &event), "payload");
}

#[test]
fn session_id_falls_back_to_the_request_metadata() {
    let event = kwargs(
        r#"{"standard_logging_object": {"session_id": "", "trace_id": "trace"},
            "litellm_params": {"metadata": {"session_id": "metadata"}}}"#,
    );
    let payload = event
        .standard_logging_object
        .as_ref()
        .expect("the fixture carries a payload");
    assert_eq!(session_id(payload, &event), "metadata");
}

#[test]
fn session_id_falls_back_to_the_payload_trace_id() {
    let event = kwargs(
        r#"{"standard_logging_object": {"session_id": "", "trace_id": "trace"},
            "litellm_params": {"metadata": {"session_id": ""}}}"#,
    );
    let payload = event
        .standard_logging_object
        .as_ref()
        .expect("the fixture carries a payload");
    assert_eq!(session_id(payload, &event), "trace");
}

#[test]
fn session_id_is_empty_without_any_candidate() {
    let event = kwargs(r#"{"standard_logging_object": {}}"#);
    let payload = event
        .standard_logging_object
        .as_ref()
        .expect("the fixture carries a payload");
    assert_eq!(session_id(payload, &event), "");
}

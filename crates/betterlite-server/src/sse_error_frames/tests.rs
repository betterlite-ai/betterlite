#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::{ErrorFields, SseFailure, exception_detail_parts, sse_error_frames, sse_error_payload};

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("valid JSON fixture")
}

fn fields(text: &str) -> ErrorFields {
    serde_json::from_str(text).expect("valid fields fixture")
}

fn failure(text: &str) -> SseFailure {
    serde_json::from_str(text).expect("valid failure fixture")
}

fn object(fields: ErrorFields) -> JsonValue {
    JsonValue::Object(fields.into_iter().collect())
}

#[rstest]
#[case::plain_string(r#""blocked by a guardrail""#, "blocked by a guardrail", None)]
#[case::flat_error(
    r#"{"error": "the model is unavailable", "detail": "kept"}"#,
    "the model is unavailable",
    Some(r#"{"error": "the model is unavailable", "detail": "kept"}"#)
)]
#[case::nested_error_message(
    r#"{"error": {"message": "nested message", "type": "bad"}}"#,
    "nested message",
    Some(r#"{"error": {"message": "nested message", "type": "bad"}}"#)
)]
#[case::top_level_message(
    r#"{"message": "top level", "other": 1}"#,
    "top level",
    Some(r#"{"message": "top level", "other": 1}"#)
)]
#[case::object_without_a_message("{}", "{}", Some("{}"))]
#[case::number_text("7", "7", None)]
fn exception_detail_parts_reads_the_four_shapes_and_the_json_text_fallback(
    #[case] detail: &str,
    #[case] message: &str,
    #[case] structured: Option<&str>,
) {
    let expected = structured.map(fields);
    assert_eq!(
        exception_detail_parts(&json(detail)),
        (message.to_owned(), expected)
    );
}

#[rstest]
#[case::status_code_wins(r#"{"status_code": 418, "code": "401"}"#, 418)]
#[case::code_is_the_fallback(r#"{"code": "429"}"#, 429)]
#[case::code_keeps_leading_zeros(r#"{"code": "0401"}"#, 401)]
#[case::empty_code(r#"{"code": ""}"#, 500)]
#[case::non_numeric_code(r#"{"code": "teapot"}"#, 500)]
#[case::code_beyond_a_status(r#"{"code": "70000"}"#, 500)]
#[case::no_status_at_all("{}", 500)]
fn sse_error_payload_resolves_the_status(#[case] body: &str, #[case] status: u16) {
    let (resolved, payload) = sse_error_payload(&failure(body));
    assert_eq!(resolved, status);
    assert_eq!(
        payload.get("code"),
        Some(&JsonValue::String(status.to_string()))
    );
}

#[rstest]
#[case::carried_type(
    r#"{"status_code": 401, "error_type": "custom_error"}"#,
    "custom_error"
)]
#[case::stringified_none(
    r#"{"status_code": 401, "error_type": "None"}"#,
    "authentication_error"
)]
#[case::unauthorized(r#"{"status_code": 401}"#, "authentication_error")]
#[case::forbidden(r#"{"status_code": 403}"#, "permission_error")]
#[case::rate_limited(r#"{"status_code": 429}"#, "rate_limit_error")]
#[case::client_error(r#"{"status_code": 400}"#, "invalid_request_error")]
#[case::server_error(r#"{"status_code": 500}"#, "internal_server_error")]
fn sse_error_payload_resolves_the_type(#[case] body: &str, #[case] error_type: &str) {
    let (_, payload) = sse_error_payload(&failure(body));
    assert_eq!(
        payload.get("type"),
        Some(&JsonValue::String(error_type.to_owned()))
    );
}

#[rstest]
#[case::carried_param(r#"{"param": "messages"}"#, Some("messages"))]
#[case::stringified_none(r#"{"param": "None"}"#, None)]
#[case::missing_param("{}", None)]
fn sse_error_payload_resolves_the_param(#[case] body: &str, #[case] param: Option<&str>) {
    let (_, payload) = sse_error_payload(&failure(body));
    let expected = param.map_or(JsonValue::Null, |text| JsonValue::String(text.to_owned()));
    assert_eq!(payload.get("param"), Some(&expected));
}

#[test]
fn sse_error_payload_defaults_a_missing_detail_message() {
    let (_, payload) = sse_error_payload(&failure(r#"{"status_code": 500}"#));
    assert_eq!(
        payload.get("message"),
        Some(&JsonValue::String(String::from(
            "Error processing stream start"
        )))
    );
}

#[test]
fn sse_error_payload_uses_the_detail_message() {
    let (_, payload) = sse_error_payload(&failure(r#"{"detail": "upstream refused"}"#));
    assert_eq!(
        payload.get("message"),
        Some(&JsonValue::String(String::from("upstream refused")))
    );
}

#[rstest]
#[case::existing_fields_only(
    r#"{"provider_specific_fields": {"provider": "openai", "code": "keep"}}"#,
    Some(r#"{"provider": "openai", "code": "keep"}"#)
)]
#[case::structured_overrides_existing(
    r#"{"detail": {"error": "blocked", "code": "guardrail"}, "provider_specific_fields": {"code": "provider", "extra": 1}}"#,
    Some(r#"{"code": "guardrail", "extra": 1, "error": "blocked"}"#)
)]
#[case::string_detail_keeps_the_existing_fields(
    r#"{"detail": "blocked", "provider_specific_fields": {"provider": "openai"}}"#,
    Some(r#"{"provider": "openai"}"#)
)]
#[case::empty_detail_and_fields_add_nothing(
    r#"{"detail": {}, "provider_specific_fields": {}}"#,
    None
)]
#[case::no_fields_at_all(r#"{"status_code": 500}"#, None)]
fn sse_error_payload_merges_the_provider_specific_fields(
    #[case] body: &str,
    #[case] merged: Option<&str>,
) {
    let (_, payload) = sse_error_payload(&failure(body));
    let expected = merged.map(|text| object(fields(text)));
    assert_eq!(payload.get("provider_specific_fields"), expected.as_ref());
}

#[test]
fn sse_error_frames_end_with_the_error_frame_then_done() {
    let error = fields(
        r#"{"code": "429", "message": "slow down", "param": null, "type": "rate_limit_error"}"#,
    );
    assert_eq!(
        sse_error_frames(&error),
        [
            String::from(
                "data: {\"error\":{\"code\":\"429\",\"message\":\"slow down\",\"param\":null,\"type\":\"rate_limit_error\"}}\n\n"
            ),
            String::from("data: [DONE]\n\n"),
        ]
    );
}

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::api::responses_stream_primitives::error_fields::{
    DEFAULT_STREAM_ERROR_MESSAGE, StreamErrorAttributes, StreamErrorFields, StreamErrorObject,
    error_event_fields, status_code_for_error_fields, stream_error_status_and_message,
};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn object(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn fields(pairs: &[(&str, Json)]) -> StreamErrorFields {
    error_event_fields(StreamErrorObject::Mapping(&object(pairs)))
}

#[test]
fn error_event_fields_reads_a_mapping() {
    let read = fields(&[
        ("message", text("slow")),
        ("type", text("rate_limit_error")),
        ("code", text("rate_limit_exceeded")),
    ]);
    assert_eq!(
        read,
        StreamErrorFields {
            message: "slow".to_owned(),
            error_type: Some("rate_limit_error".to_owned()),
            code: Some("rate_limit_exceeded".to_owned()),
        }
    );
}

#[test]
fn error_event_fields_reads_attributes() {
    let message = text("slow");
    let code = number(7);
    let read = error_event_fields(StreamErrorObject::Attributes(StreamErrorAttributes {
        message: Some(&message),
        error_type: None,
        code: Some(&code),
    }));
    assert_eq!(
        read,
        StreamErrorFields {
            message: "slow".to_owned(),
            error_type: None,
            code: Some("7".to_owned()),
        }
    );
}

#[rstest]
#[case::absent(None, DEFAULT_STREAM_ERROR_MESSAGE)]
#[case::null(Some(Json::Null), DEFAULT_STREAM_ERROR_MESSAGE)]
#[case::a_number(Some(number(7)), "7")]
#[case::an_empty_message(Some(text("")), "")]
#[case::a_bool(Some(Json::Bool(true)), "True")]
#[case::a_string(Some(text("slow")), "slow")]
fn error_event_fields_stringifies_the_message(#[case] given: Option<Json>, #[case] expected: &str) {
    let mut pairs = Vec::new();
    if let Some(value) = given {
        pairs.push(("message", value));
    }
    assert_eq!(fields(&pairs).message, expected);
}

#[rstest]
#[case::a_string(Some(text("rate_limit_error")), Some("rate_limit_error"))]
#[case::a_number(Some(number(1)), None)]
#[case::a_bool(Some(Json::Bool(true)), None)]
#[case::null(Some(Json::Null), None)]
#[case::an_array(Some(Json::List(Vec::new())), None)]
fn error_event_fields_keeps_only_a_string_type(
    #[case] given: Option<Json>,
    #[case] expected: Option<&str>,
) {
    let mut pairs = Vec::new();
    if let Some(value) = given {
        pairs.push(("type", value));
    }
    assert_eq!(fields(&pairs).error_type.as_deref(), expected);
}

#[rstest]
#[case::a_string(Some(text("server_error")), Some("server_error"))]
#[case::an_integer(Some(number(7)), Some("7"))]
#[case::an_empty_string(Some(text("")), Some(""))]
#[case::a_bool(Some(Json::Bool(true)), None)]
#[case::null(Some(Json::Null), None)]
#[case::a_float(Some(Json::Number(Number::from_f64(1.5).expect("1.5 is finite"))), None)]
#[case::a_whole_float(Some(Json::Number(Number::from_f64(1.0).expect("1.0 is finite"))), None)]
#[case::an_object(Some(Json::Object(IndexMap::new())), None)]
fn error_event_fields_keeps_a_string_or_integer_code(
    #[case] given: Option<Json>,
    #[case] expected: Option<&str>,
) {
    let mut pairs = Vec::new();
    if let Some(value) = given {
        pairs.push(("code", value));
    }
    assert_eq!(fields(&pairs).code.as_deref(), expected);
}

#[rstest]
#[case::a_rate_limit_code(None, Some("rate_limit_exceeded"), 429)]
#[case::a_rate_limit_type(Some("rate_limit_error"), None, 429)]
#[case::an_insufficient_quota_type(Some("insufficient_quota"), None, 429)]
#[case::an_insufficient_quota_code(None, Some("insufficient_quota"), 429)]
#[case::a_prefixed_type_over_a_conflicting_code(Some("rate_limit_error"), Some("500"), 429)]
#[case::a_prefixed_code_over_a_conflicting_type(Some("server_error"), Some("rate_limited"), 429)]
fn a_rate_limit_field_wins(
    #[case] error_type: Option<&str>,
    #[case] code: Option<&str>,
    #[case] expected: u16,
) {
    assert_eq!(status_code_for_error_fields(error_type, code), expected);
}

#[rstest]
#[case::the_code_wins(Some("server_error"), Some("401"), 401)]
#[case::a_decimal_code_beats_a_symbolic_type(Some("model_not_found"), Some("500"), 500)]
#[case::the_type_is_the_fallback(Some("server_error"), None, 500)]
#[case::an_unknown_type_falls_back_to_500(Some("bad_request"), None, 500)]
#[case::no_fields(None, None, 500)]
fn the_code_is_read_before_the_type(
    #[case] error_type: Option<&str>,
    #[case] code: Option<&str>,
    #[case] expected: u16,
) {
    assert_eq!(status_code_for_error_fields(error_type, code), expected);
}

#[rstest]
#[case::the_lowest_status(Some("400"), 400)]
#[case::the_highest_status(Some("599"), 599)]
#[case::a_padded_status(Some("0400"), 400)]
#[case::below_the_range(Some("399"), 500)]
#[case::above_the_range(Some("600"), 500)]
#[case::not_a_number(Some("4o0"), 500)]
fn a_decimal_field_becomes_a_status_only_inside_the_range(
    #[case] code: Option<&str>,
    #[case] expected: u16,
) {
    assert_eq!(status_code_for_error_fields(None, code), expected);
}

#[test]
fn every_named_invalid_request_code_maps_to_400() {
    let codes = [
        "invalid_prompt",
        "invalid_image",
        "invalid_image_format",
        "invalid_base64_image",
        "invalid_image_url",
        "image_too_large",
        "image_too_small",
        "image_parse_error",
        "image_content_policy_violation",
        "invalid_image_mode",
        "image_file_too_large",
        "unsupported_image_media_type",
        "empty_image_file",
        "failed_to_download_image",
        "image_file_not_found",
        "invalid_request_error",
        "context_length_exceeded",
        "content_policy_violation",
        "model_not_found",
    ];
    for code in codes {
        assert_eq!(
            status_code_for_error_fields(None, Some(code)),
            400,
            "{code}"
        );
    }
}

#[rstest]
#[case::server_error(Some("server_error"), 500)]
#[case::rate_limit_exceeded(Some("rate_limit_exceeded"), 429)]
#[case::vector_store_timeout(Some("vector_store_timeout"), 504)]
#[case::an_unknown_code(Some("mystery"), 500)]
fn a_symbolic_code_maps_through_the_table(#[case] code: Option<&str>, #[case] expected: u16) {
    assert_eq!(status_code_for_error_fields(None, code), expected);
}

#[test]
fn stream_error_status_and_message_reports_both() {
    let mapping = object(&[
        ("message", text("slow")),
        ("code", text("rate_limit_exceeded")),
    ]);
    assert_eq!(
        stream_error_status_and_message(StreamErrorObject::Mapping(&mapping)),
        (429, "slow".to_owned())
    );
    let message = text("late");
    let code = text("vector_store_timeout");
    assert_eq!(
        stream_error_status_and_message(StreamErrorObject::Attributes(StreamErrorAttributes {
            message: Some(&message),
            error_type: None,
            code: Some(&code),
        })),
        (504, "late".to_owned())
    );
}

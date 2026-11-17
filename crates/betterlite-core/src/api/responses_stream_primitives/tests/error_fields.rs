use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::api::responses_stream_primitives::error_fields::{
    DEFAULT_STREAM_ERROR_MESSAGE, StreamErrorAttributes, StreamErrorFields, StreamErrorObject,
    error_code_status, error_event_fields, error_status_and_message, status_code_for_error_fields,
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
    let read = given.map_or_else(|| fields(&[]), |value| fields(&[("type", value)]));
    assert_eq!(read.error_type.as_deref(), expected);
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
    let read = given.map_or_else(|| fields(&[]), |value| fields(&[("code", value)]));
    assert_eq!(read.code.as_deref(), expected);
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
#[case::a_signed_number(Some("+400"), 500)]
fn a_decimal_field_becomes_a_status_only_inside_the_range(
    #[case] code: Option<&str>,
    #[case] expected: u16,
) {
    assert_eq!(status_code_for_error_fields(None, code), expected);
}

// str.isdecimal() takes the Arabic-Indic digits and int() reads them as 429, which is a documented deviation
#[test]
fn a_non_ascii_decimal_code_is_not_a_status() {
    let code: String = [0x0664_u32, 0x0662, 0x0669]
        .into_iter()
        .filter_map(char::from_u32)
        .collect();
    assert_eq!(status_code_for_error_fields(None, Some(&code)), 500);
}

#[rstest]
#[case::invalid_prompt("invalid_prompt")]
#[case::invalid_image("invalid_image")]
#[case::invalid_image_format("invalid_image_format")]
#[case::invalid_base64_image("invalid_base64_image")]
#[case::invalid_image_url("invalid_image_url")]
#[case::image_too_large("image_too_large")]
#[case::image_too_small("image_too_small")]
#[case::image_parse_error("image_parse_error")]
#[case::image_content_policy_violation("image_content_policy_violation")]
#[case::invalid_image_mode("invalid_image_mode")]
#[case::image_file_too_large("image_file_too_large")]
#[case::unsupported_image_media_type("unsupported_image_media_type")]
#[case::empty_image_file("empty_image_file")]
#[case::failed_to_download_image("failed_to_download_image")]
#[case::image_file_not_found("image_file_not_found")]
#[case::invalid_request_error("invalid_request_error")]
#[case::context_length_exceeded("context_length_exceeded")]
#[case::content_policy_violation("content_policy_violation")]
#[case::model_not_found("model_not_found")]
fn every_named_invalid_request_code_maps_to_400(#[case] code: &str) {
    assert_eq!(
        status_code_for_error_fields(None, Some(code)),
        400,
        "{code}"
    );
}

#[rstest]
#[case::server_error(Some("server_error"), 500)]
#[case::rate_limit_exceeded(Some("rate_limit_exceeded"), 429)]
#[case::vector_store_timeout(Some("vector_store_timeout"), 504)]
#[case::an_unknown_code(Some("mystery"), 500)]
fn a_symbolic_code_maps_through_the_table(#[case] code: Option<&str>, #[case] expected: u16) {
    assert_eq!(status_code_for_error_fields(None, code), expected);
}

#[rstest]
#[case::server_error("server_error", Some(500))]
#[case::rate_limit_exceeded("rate_limit_exceeded", Some(429))]
#[case::insufficient_quota("insufficient_quota", Some(429))]
#[case::vector_store_timeout("vector_store_timeout", Some(504))]
#[case::an_unknown_symbol("mystery", None)]
fn the_table_answers_each_named_code(#[case] field: &str, #[case] expected: Option<u16>) {
    assert_eq!(error_code_status(field), expected);
}

#[test]
fn error_status_and_message_reports_both() {
    let mapping = object(&[
        ("message", text("slow")),
        ("code", text("rate_limit_exceeded")),
    ]);
    assert_eq!(
        error_status_and_message(StreamErrorObject::Mapping(&mapping)),
        (429, "slow".to_owned())
    );
    let message = text("late");
    let code = text("vector_store_timeout");
    assert_eq!(
        error_status_and_message(StreamErrorObject::Attributes(StreamErrorAttributes {
            message: Some(&message),
            error_type: None,
            code: Some(&code),
        })),
        (504, "late".to_owned())
    );
}

use rstest::rstest;

use super::support::json;
use crate::api::responses_parameters::error::ResponsesParametersError;
use crate::api::responses_parameters::text_format::text_format_to_text;

const FORMAT: &str = r#"{"type": "json_schema", "json_schema": {"name": "X", "schema": {"type": "object"}, "strict": true}}"#;

#[test]
fn a_response_format_becomes_the_responses_text_shape() {
    let format = json(FORMAT);
    let converted = text_format_to_text(Some(&format), None).expect("a complete format converts");
    assert_eq!(
        converted,
        Some(json(
            r#"{"format": {"type": "json_schema", "name": "X", "schema": {"type": "object"}, "strict": true}}"#
        ))
    );
}

#[rstest]
#[case::empty_object("{}")]
#[case::non_empty(r#"{"format": {"type": "json_schema"}}"#)]
fn an_existing_text_wins(#[case] existing: &str) {
    let format = json(FORMAT);
    let text = json(existing);
    let converted = text_format_to_text(Some(&format), Some(&text)).expect("text is returned");
    assert_eq!(converted, Some(text));
}

#[test]
fn a_null_text_format_returns_the_text_unchanged() {
    assert_eq!(text_format_to_text(None, None).expect("no format"), None);
    let text = json("{}");
    assert_eq!(
        text_format_to_text(None, Some(&text)).expect("no format"),
        Some(text)
    );
}

#[rstest]
#[case::not_an_object(r#""json_schema""#, "text_format must be an object")]
#[case::missing_type(
    r#"{"json_schema": {"name": "X", "schema": {}, "strict": true}}"#,
    "text_format has no type"
)]
#[case::missing_json_schema(r#"{"type": "json_schema"}"#, "text_format has no json_schema")]
#[case::missing_name(
    r#"{"type": "json_schema", "json_schema": {"schema": {}, "strict": true}}"#,
    "json_schema.name"
)]
#[case::missing_schema(
    r#"{"type": "json_schema", "json_schema": {"name": "X", "strict": true}}"#,
    "json_schema.schema"
)]
#[case::missing_strict(
    r#"{"type": "json_schema", "json_schema": {"name": "X", "schema": {}}}"#,
    "json_schema.strict"
)]
fn an_incomplete_format_is_an_error(#[case] source: &str, #[case] expected: &str) {
    let format = json(source);
    let error = text_format_to_text(Some(&format), None).expect_err("an incomplete format fails");
    assert!(error.to_string().contains(expected), "{error}");
    assert!(matches!(
        error,
        ResponsesParametersError::TextFormatNotObject
            | ResponsesParametersError::MissingTextFormatField { .. }
    ));
}

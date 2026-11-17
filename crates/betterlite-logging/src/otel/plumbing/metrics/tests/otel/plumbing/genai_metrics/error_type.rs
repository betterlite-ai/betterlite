use betterlite_logging::otel::plumbing::metrics::error_type::{
    ERROR_TYPE_FALLBACK, resolve_error_type,
};
use rstest::rstest;

use super::support::kwargs;

fn error_type(extra: &str) -> String {
    resolve_error_type(&kwargs(extra)).expect("error type")
}

#[test]
fn the_mapped_error_class_wins_over_the_error_code() {
    let extra = r#"{"standard_logging_object": {"error_information": {"error_class": "RateLimitError", "error_code": "429"}}}"#;
    assert_eq!(error_type(extra), "RateLimitError");
}

#[rstest]
#[case::class_absent(r#"{"error_code": "429"}"#, "429")]
#[case::class_null(r#"{"error_class": null, "error_code": "429"}"#, "429")]
fn the_error_code_is_used_when_the_class_is_absent_or_null(
    #[case] information: &str,
    #[case] expected: &str,
) {
    let extra = format!(r#"{{"standard_logging_object": {{"error_information": {information}}}}}"#);
    assert_eq!(error_type(&extra), expected);
}

#[rstest]
#[case::code(r#"{"error_code": 429}"#, "429")]
#[case::class(r#"{"error_class": 429}"#, "429")]
fn an_error_value_that_is_not_text_is_serialized(
    #[case] information: &str,
    #[case] expected: &str,
) {
    let extra = format!(r#"{{"standard_logging_object": {{"error_information": {information}}}}}"#);
    assert_eq!(error_type(&extra), expected);
}

#[test]
fn the_exception_class_name_is_used_without_error_information() {
    let extra = r#"{"exception": {"class_name": "APIError", "message": "boom"}}"#;
    assert_eq!(error_type(extra), "APIError");
}

#[test]
fn an_empty_error_information_falls_through_to_the_exception() {
    let extra = r#"{"standard_logging_object": {}, "exception": {"class_name": "APIError"}}"#;
    assert_eq!(error_type(extra), "APIError");
}

#[test]
fn error_information_wins_over_the_exception() {
    let extra = r#"{
        "standard_logging_object": {"error_information": {"error_class": "RateLimitError"}},
        "exception": {"class_name": "APIError"}
    }"#;
    assert_eq!(error_type(extra), "RateLimitError");
}

#[rstest]
#[case::nothing("{}")]
#[case::exception_without_a_class(r#"{"exception": {}}"#)]
#[case::exception_with_an_empty_class(r#"{"exception": {"class_name": ""}}"#)]
fn the_sentinel_is_the_error_type_when_nothing_classifies(#[case] extra: &str) {
    assert_eq!(error_type(extra), ERROR_TYPE_FALLBACK);
}

use std::collections::BTreeMap;

use super::support::{DURATION, attributes_of, fail_with, failure, late_settings, names, sums};

#[test]
fn a_failure_records_only_the_operation_duration_with_the_error_type() {
    let extra = r#"{"standard_logging_object": {"error_information": {"error_class": "RateLimitError", "error_code": "429"}}}"#;
    let recorded = failure(extra);
    assert_eq!(names(&recorded), [DURATION]);
    assert_eq!(sums(&recorded, DURATION), ["3"]);
    let attributes = attributes_of(&recorded, DURATION);
    assert_eq!(attributes["gen_ai.operation.name"], "chat");
    assert_eq!(attributes["error.type"], "RateLimitError");
}

#[test]
fn a_failure_nothing_can_classify_is_tagged_with_the_sentinel() {
    let recorded = failure("{}");
    assert_eq!(attributes_of(&recorded, DURATION)["error.type"], "_OTHER");
}

#[test]
fn the_exception_class_name_is_the_error_type_without_error_information() {
    let extra = r#"{
        "standard_logging_object": {},
        "exception": {"class_name": "APIConnectionError", "message": "boom"}
    }"#;
    let recorded = failure(extra);
    assert_eq!(
        attributes_of(&recorded, DURATION)["error.type"],
        "APIConnectionError"
    );
}

#[test]
fn a_failure_with_a_cost_stream_and_timestamps_still_records_only_the_duration() {
    let extra = r#"{
        "response_cost": 0.5,
        "optional_params": {"stream": true},
        "api_call_start_time": 10.5,
        "completion_start_time": 11.0
    }"#;
    let recorded = failure(extra);
    assert_eq!(names(&recorded), [DURATION]);
}

#[test]
fn an_exclude_list_cannot_strip_the_error_type() {
    let settings = late_settings(Some(
        r#"{"exclude_list": ["gen_ai.framework", "hidden_params"]}"#,
    ));
    let extra =
        r#"{"standard_logging_object": {"error_information": {"error_class": "APIError"}}}"#;
    let recorded = fail_with(settings, None, extra);
    let attributes = attributes_of(&recorded, DURATION);
    assert_eq!(attributes["error.type"], "APIError");
    assert!(!attributes.contains_key("gen_ai.framework"));
    assert!(!attributes.contains_key("hidden_params"));
}

#[test]
fn an_include_list_cannot_strip_the_error_type() {
    let settings = late_settings(Some(r#"{"include_list": ["gen_ai.system"]}"#));
    let extra =
        r#"{"standard_logging_object": {"error_information": {"error_class": "APIError"}}}"#;
    let recorded = fail_with(settings, None, extra);
    let expected = BTreeMap::from([
        ("error.type".to_owned(), "APIError".to_owned()),
        ("gen_ai.system".to_owned(), "openai".to_owned()),
    ]);
    assert_eq!(attributes_of(&recorded, DURATION), expected);
}

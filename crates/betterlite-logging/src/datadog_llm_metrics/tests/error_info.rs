#![cfg(test)]

use crate::datadog_llm_metrics::error_info::error_info;
use crate::datadog_llm_metrics::tests::support;
use crate::datadog_llm_privacy::record_object;
use crate::fields::status::LoggingStatus;

#[test]
fn a_failure_payload_carries_message_class_and_stack() {
    let mut payload = support::payload();
    payload.status = LoggingStatus::Failure;
    payload.error_information = Some(support::error_information(
        r#"{"error_message": "boom", "error_class": "BadRequestError", "traceback": "the traceback"}"#,
    ));
    let error = error_info(&payload).expect("the failure carries an error");
    let object = record_object("error", &error).expect("the error encodes");
    assert_eq!(
        object,
        support::object(
            r#"{"message": "boom", "stack": "the traceback", "type": "BadRequestError"}"#
        )
    );
}

#[test]
fn a_missing_message_falls_back_to_the_error_string() {
    let mut payload = support::payload();
    payload.status = LoggingStatus::Failure;
    payload.error_information = Some(support::error_information(r#"{"error_class": "APIError"}"#));
    payload.error_str = Some("the provider refused".to_owned());
    let error = error_info(&payload).expect("the failure carries an error");
    assert_eq!(error.message.as_deref(), Some("the provider refused"));
}

#[test]
fn a_null_message_falls_back_to_the_error_string() {
    let mut payload = support::payload();
    payload.status = LoggingStatus::Failure;
    payload.error_information = Some(support::error_information(r#"{"error_message": null}"#));
    payload.error_str = Some("the provider refused".to_owned());
    let error = error_info(&payload).expect("the failure carries an error");
    assert_eq!(error.message.as_deref(), Some("the provider refused"));
}

#[test]
fn a_missing_error_string_uses_unknown_error() {
    let mut payload = support::payload();
    payload.status = LoggingStatus::Failure;
    payload.error_information = Some(support::error_information("{}"));
    let error = error_info(&payload).expect("the failure carries an error");
    assert_eq!(error.message.as_deref(), Some("Unknown error"));
}

#[test]
fn an_empty_message_is_used_as_given() {
    let mut payload = support::payload();
    payload.status = LoggingStatus::Failure;
    payload.error_information = Some(support::error_information(r#"{"error_message": ""}"#));
    payload.error_str = Some("the provider refused".to_owned());
    let error = error_info(&payload).expect("the failure carries an error");
    assert_eq!(error.message.as_deref(), Some(""));
}

#[test]
fn a_null_error_class_and_traceback_serialize_as_null() {
    let mut payload = support::payload();
    payload.status = LoggingStatus::Failure;
    payload.error_information = Some(support::error_information(
        r#"{"error_message": "boom", "error_class": null, "traceback": null}"#,
    ));
    let error = error_info(&payload).expect("the failure carries an error");
    let object = record_object("error", &error).expect("the error encodes");
    assert_eq!(
        object,
        support::object(r#"{"message": "boom", "stack": null, "type": null}"#)
    );
}

#[test]
fn a_success_payload_has_no_error() {
    let mut payload = support::payload();
    payload.error_information = Some(support::error_information(r#"{"error_message": "boom"}"#));
    assert!(error_info(&payload).is_none());
}

#[test]
fn a_failure_without_error_information_has_no_error() {
    let mut payload = support::payload();
    payload.status = LoggingStatus::Failure;
    payload.error_str = Some("the provider refused".to_owned());
    assert!(error_info(&payload).is_none());
}

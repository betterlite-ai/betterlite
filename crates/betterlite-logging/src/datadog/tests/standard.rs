use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::datadog::apm::NullTracer;
use crate::datadog::payloads::standard::standard_logging_to_datadog;
use crate::datadog::payloads::status::DataDogStatus;
use crate::exporter_options::truncation::{MAX_LOGGED_TEXT_CHARS, TRUNCATION_SUFFIX};
use crate::fields::status::LoggingStatus;

use super::support::{
    default_environment, environment, field, json, message, standard_payload, tagged_payload,
};

#[derive(Clone, Copy, Debug)]
enum LongField {
    ErrorStr,
    Messages,
    Response,
}

fn text_len(value: &Json) -> Option<usize> {
    match value {
        Json::Text(text) => Some(text.len()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

#[test]
fn datadog_standard_logging_behavior() {
    let environment = environment(&[
        ("DD_SOURCE", "dd-source"),
        ("DD_SERVICE", "dd-service"),
        ("HOSTNAME", "host"),
        ("DD_ENV", "staging"),
        ("POD_NAME", "pod"),
        ("DD_VERSION", "1.2.3"),
    ]);
    let built = standard_logging_to_datadog(
        &tagged_payload(),
        DataDogStatus::Warn,
        &environment,
        &NullTracer,
    )
    .unwrap();
    assert_eq!(built.ddsource, "dd-source");
    assert_eq!(built.service, "dd-service");
    assert_eq!(built.hostname, "host");
    assert_eq!(built.status, DataDogStatus::Warn);
    let tags = "env:staging,service:dd-service,version:1.2.3,HOSTNAME:host,POD_NAME:pod";
    assert_eq!(
        built.ddtags,
        format!("{tags},request_tag:prod_a,request_tag:7,team:team_one")
    );
    let body = message(&built);
    assert_eq!(field(&body, "id"), &json(r#""chatcmpl-1""#));
    assert_eq!(field(&body, "model"), &json(r#""gpt-4o""#));
    assert_eq!(field(&body, "response_cost"), &json("0.5"));
    assert_eq!(
        DataDogStatus::from_logging_status(LoggingStatus::Failure),
        DataDogStatus::Error
    );
    assert_eq!(
        DataDogStatus::from_logging_status(LoggingStatus::Success),
        DataDogStatus::Info
    );
}

#[rstest]
#[case::error_str(LongField::ErrorStr)]
#[case::messages(LongField::Messages)]
#[case::response(LongField::Response)]
fn datadog_standard_logging_truncates_long_fields(#[case] selected: LongField) {
    let long = "x".repeat(MAX_LOGGED_TEXT_CHARS + 1);
    let mut payload = standard_payload();
    match selected {
        LongField::ErrorStr => payload.error_str = Some(long),
        LongField::Messages => payload.messages = Some(Json::Text(long)),
        LongField::Response => payload.response = Some(Json::Text(long)),
    }
    let built = standard_logging_to_datadog(
        &payload,
        DataDogStatus::Info,
        &default_environment(),
        &NullTracer,
    )
    .unwrap();
    let mut expected = "x".repeat(MAX_LOGGED_TEXT_CHARS);
    expected.push_str(TRUNCATION_SUFFIX);
    let name = match selected {
        LongField::ErrorStr => "error_str",
        LongField::Messages => "messages",
        LongField::Response => "response",
    };
    assert_eq!(field(&message(&built), name), &Json::Text(expected));
    let source = match selected {
        LongField::ErrorStr => payload.error_str.as_ref().map(String::len),
        LongField::Messages => payload.messages.as_ref().and_then(text_len),
        LongField::Response => payload.response.as_ref().and_then(text_len),
    };
    assert_eq!(source, Some(MAX_LOGGED_TEXT_CHARS + 1));
}

#[rstest]
#[case::text(r#""Prod A""#, "request_tag:prod_a")]
#[case::number("7", "request_tag:7")]
#[case::null("null", "request_tag:none")]
#[case::boolean("true", "request_tag:true")]
#[case::object(r#"{"team": "A B"}"#, "request_tag:team_:_a_b")]
#[case::list("[1, 2]", "request_tag:1_2")]
fn datadog_standard_logging_maps_request_tag_values(#[case] tag: &str, #[case] expected: &str) {
    let mut payload = standard_payload();
    payload.request_tags = vec![json(tag)];
    let built = standard_logging_to_datadog(
        &payload,
        DataDogStatus::Info,
        &default_environment(),
        &NullTracer,
    )
    .unwrap();
    assert!(built.ddtags.contains(expected));
}

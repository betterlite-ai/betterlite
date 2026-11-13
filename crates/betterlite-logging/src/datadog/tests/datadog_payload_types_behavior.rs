use rstest::rstest;

use crate::datadog::payloads::constants::DATADOG_413_ERROR;
use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::payloads::types::DatadogPayload;
use crate::datadog::tests::support::{field, has_key, json};
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

fn base_payload() -> DatadogPayload {
    DatadogPayload {
        ddsource: "litellm".to_owned(),
        ddtags: "env:unknown".to_owned(),
        hostname: String::new(),
        message: "{}".to_owned(),
        service: "litellm-server".to_owned(),
        status: DataDogStatus::Info,
        trace_id: None,
        span_id: None,
    }
}

#[rstest]
#[case::info(DataDogStatus::Info, r#""info""#)]
#[case::warn(DataDogStatus::Warn, r#""warning""#)]
#[case::error(DataDogStatus::Error, r#""error""#)]
fn status_values(#[case] status: DataDogStatus, #[case] expected: &str) {
    assert_eq!(serde_json::to_string(&status).unwrap(), expected);
}

#[test]
fn serializes_its_required_fields() {
    let encoded = safe_dumps(&base_payload(), DEFAULT_MAX_RECURSE_DEPTH, None).unwrap();
    let parsed = json(&encoded);
    assert_eq!(field(&parsed, "ddsource"), &json(r#""litellm""#));
    assert_eq!(field(&parsed, "ddtags"), &json(r#""env:unknown""#));
    assert_eq!(field(&parsed, "hostname"), &json(r#""""#));
    assert_eq!(field(&parsed, "message"), &json(r#""{}""#));
    assert_eq!(field(&parsed, "service"), &json(r#""litellm-server""#));
    assert_eq!(field(&parsed, "status"), &json(r#""info""#));
    assert!(!has_key(&parsed, "dd.trace_id"));
    assert!(!has_key(&parsed, "dd.span_id"));
}

#[test]
fn serializes_the_trace_fields_when_present() {
    let mut payload = base_payload();
    payload.trace_id = Some("12".to_owned());
    payload.span_id = Some("34".to_owned());
    let encoded = safe_dumps(&payload, DEFAULT_MAX_RECURSE_DEPTH, None).unwrap();
    let parsed = json(&encoded);
    assert_eq!(field(&parsed, "dd.trace_id"), &json(r#""12""#));
    assert_eq!(field(&parsed, "dd.span_id"), &json(r#""34""#));
}

#[test]
fn carries_the_413_error_message() {
    let expected = "Datadog API Error - Payload too large (batch is above 5MB uncompressed). If you want this logged either disable request/response logging or set `DD_BATCH_SIZE=50`";
    assert_eq!(DATADOG_413_ERROR, expected);
}

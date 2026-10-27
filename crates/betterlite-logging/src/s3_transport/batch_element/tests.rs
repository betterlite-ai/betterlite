#![cfg(test)]

use rstest::rstest;
use serde::{Deserialize, Serialize};

use crate::s3_transport::batch_element::S3BatchLoggingElement;

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct Payload {
    id: String,
    response: Option<String>,
}

#[rstest]
#[case::defaults_fill_missing_fields(
    r#"{"payload":{"id":"x","response":null},"s3_object_key":"k","s3_object_download_filename":"f.json"}"#,
    r#"{"payload":{"id":"x","response":null},"s3_object_key":"k","s3_object_download_filename":"f.json","body":null,"content_type":"application/json","retrying_since":null}"#
)]
#[case::every_field_survives(
    r#"{"payload":{"id":"x","response":"y"},"s3_object_key":"k","s3_object_download_filename":"f.json","body":"{}","content_type":"application/x-ndjson","retrying_since":1.5}"#,
    r#"{"payload":{"id":"x","response":"y"},"s3_object_key":"k","s3_object_download_filename":"f.json","body":"{}","content_type":"application/x-ndjson","retrying_since":1.5}"#
)]
fn s3_option_resolution_behaviors_behavior(#[case] stored: &str, #[case] expected: &str) {
    let element: S3BatchLoggingElement<Payload> = serde_json::from_str(stored).unwrap();
    let encoded = serde_json::to_string(&element).unwrap();
    assert_eq!(encoded, expected);
    let decoded: S3BatchLoggingElement<Payload> = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, element);
}

#![cfg(test)]

use super::{EndpointType, PassthroughStandardLoggingPayload, get_endpoint_type};
use serde_json::Value;

#[test]
fn passthrough_endpoint_type_behavior() {
    assert_eq!(
        get_endpoint_type("https://vertex.example/generateContent", None),
        EndpointType::VertexAi
    );
}

#[test]
fn passthrough_payload() {
    let _payload = PassthroughStandardLoggingPayload {
        url: "https://example.com".to_owned(),
        request_method: Some("POST".to_owned()),
        request_body: Some(serde_json::from_str::<Value>(r#"{"input":1}"#).unwrap()),
        response_body: None,
        cost_per_request: Some(0.25_f64),
    };
}

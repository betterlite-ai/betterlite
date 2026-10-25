#![cfg(test)]

use super::{
    CUSTOM_BODY_STATE_KEY, ENDPOINT_MARKER, EndpointType, MODEL_INFO_STATE_KEY,
    PassthroughLogPayload, RAW_BODY_STATE_KEY, get_endpoint_type,
};
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::json;

#[rstest]
#[case::vertex(
    "https://vertex.example/v1/m:generateContent",
    None,
    EndpointType::VertexAi
)]
#[case::vertex_stream(
    "https://g.example/m:streamGenerateContent",
    None,
    EndpointType::VertexAi
)]
#[case::vertex_raw("https://vertex.example/m:rawPredict", None, EndpointType::VertexAi)]
#[case::vertex_stream_raw(
    "https://vertex.example/m:streamRawPredict",
    None,
    EndpointType::VertexAi
)]
#[case::anthropic("https://API.anthropic.com/v1/messages", None, EndpointType::Anthropic)]
#[case::openai("https://api.openai.com/chat", None, EndpointType::OpenAi)]
#[case::azure("https://openai.azure.com/chat", None, EndpointType::OpenAi)]
#[case::openai_substring("https://eu.notopenai.com.example/chat", None, EndpointType::OpenAi)]
#[case::tinyfish("https://agent.tinyfish.ai/runs", None, EndpointType::Generic)]
#[case::tinyfish_subdomain("https://eu.agent.tinyfish.ai/runs", None, EndpointType::Generic)]
#[case::generic("https://example.test/infer", None, EndpointType::Generic)]
#[case::unparsable("not a url", None, EndpointType::Generic)]
#[case::decisions(
    "https://example.test/v1/systemone/?q=1",
    Some("typesafe"),
    EndpointType::Decisions
)]
#[case::decisions_precede_vertex(
    "https://example.test/v1/systemone?next=generateContent",
    Some("typesafe"),
    EndpointType::Decisions
)]
#[case::decisions_two_slashes(
    "https://example.test/v1/systemone//",
    Some("typesafe"),
    EndpointType::Generic
)]
#[case::wrong_provider(
    "https://example.test/v1/systemone",
    Some("other"),
    EndpointType::Generic
)]
fn passthrough_endpoint_type_behavior(
    #[case] url: &str,
    #[case] provider: Option<&str>,
    #[case] expected: EndpointType,
) {
    assert_eq!(get_endpoint_type(url, provider), expected);
}

#[test]
fn passthrough_payload() {
    let payload = PassthroughLogPayload {
        url: "https://up/v1".to_owned(),
        request_method: Some("POST".to_owned()),
        request_body: Some(serde_json::from_str(r#"{"a":1}"#).unwrap()),
        response_body: None,
        cost_per_request: Some(Decimal::new(5, 1)),
    };
    let expected: serde_json::Value =
        serde_json::from_str(r#"{"url":"https://up/v1","request_method":"POST","request_body":{"a":1},"response_body":null,"cost_per_request":0.5}"#).unwrap();
    assert_eq!(serde_json::to_value(payload).unwrap(), expected);
}

#[test]
fn passthrough_payload_variants_and_state_keys() {
    let endpoint_types = json!([
        EndpointType::VertexAi,
        EndpointType::Gemini,
        EndpointType::Anthropic,
        EndpointType::OpenAi,
        EndpointType::Tinyfish,
        EndpointType::Generic,
        EndpointType::Decisions
    ]);
    assert_eq!(
        endpoint_types,
        json!([
            "vertex-ai",
            "gemini",
            "anthropic",
            "openai",
            "tinyfish",
            "generic",
            "decisions"
        ])
    );
    assert_eq!(
        [
            CUSTOM_BODY_STATE_KEY,
            RAW_BODY_STATE_KEY,
            MODEL_INFO_STATE_KEY,
            ENDPOINT_MARKER
        ],
        [
            "litellm_pass_through_custom_body",
            "litellm_pass_through_raw_body",
            "litellm_pass_through_deployment_model_info",
            "__litellm_pass_through_endpoint__"
        ]
    );
}

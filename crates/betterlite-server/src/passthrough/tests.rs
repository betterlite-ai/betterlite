#![cfg(test)]

use super::{EndpointType, PassthroughLogPayload, get_endpoint_type};
use rstest::rstest;

#[rstest]
#[case::vertex("https://vertex.example/generateContent", None, EndpointType::VertexAi)]
#[case::anthropic("https://api.anthropic.com/v1/messages", None, EndpointType::Anthropic)]
#[case::openai("https://api.openai.com/chat", None, EndpointType::OpenAi)]
#[case::tinyfish("https://agent.tinyfish.ai/runs", None, EndpointType::Tinyfish)]
#[case::generic("https://example.test/infer", None, EndpointType::Generic)]
#[case::decisions(
    "https://example.test/v1/systemone",
    Some("typesafe"),
    EndpointType::Decisions
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
    let _payload = PassthroughLogPayload {
        url: String::new(),
        request_method: None,
        request_body: None,
        response_body: None,
        cost_per_request: None,
    };
}

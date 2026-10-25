#![cfg(test)]

use rstest::rstest;

use crate::otel::vocabulary::constants::{
    GenAIOperation, GenAIOutputType, GenAIProvider, MCPMethod, NetworkTransport, RpcSystem,
};

#[rstest]
#[case::text_completion(GenAIOperation::TextCompletion.to_string(), "text_completion")]
#[case::create_agent(GenAIOperation::CreateAgent.to_string(), "create_agent")]
#[case::vendor_file_management(
    GenAIOperation::LitellmVectorStoreFileManagement.to_string(),
    "litellm.vector_store_file_management"
)]
#[case::json_output(GenAIOutputType::Json.to_string(), "json")]
#[case::mistral(GenAIProvider::MistralAi.to_string(), "mistral_ai")]
#[case::xai(GenAIProvider::XAi.to_string(), "x_ai")]
#[case::vertex(GenAIProvider::GcpVertexAi.to_string(), "gcp.vertex_ai")]
#[case::watsonx(GenAIProvider::IbmWatsonxAi.to_string(), "ibm.watsonx.ai")]
#[case::prompts_get(MCPMethod::PromptsGet.to_string(), "prompts/get")]
#[case::jsonrpc(RpcSystem::JsonRpc.to_string(), "jsonrpc")]
#[case::quic(NetworkTransport::Quic.to_string(), "quic")]
fn enum_values_are_spelled_as_litellm_sends_them(#[case] value: String, #[case] expected: &str) {
    assert_eq!(value, expected);
}

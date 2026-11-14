mod inference;

use rstest::rstest;

use crate::agent_policy::routes::managed_agent_route_allowed;

#[rstest]
#[case::agents("/agents")]
#[case::v1_agents("/v1/agents")]
fn route_agent_listing_reads_only(#[case] route: &str) {
    assert!(managed_agent_route_allowed(route, None));
    assert!(managed_agent_route_allowed(route, Some("GET")));
    assert!(managed_agent_route_allowed(route, Some("HEAD")));
    assert!(!managed_agent_route_allowed(route, Some("POST")));
    assert!(!managed_agent_route_allowed(route, Some("get")));
}

#[rstest]
#[case::realtime("/realtime")]
#[case::v1_realtime("/v1/realtime")]
#[case::openai_realtime("/openai/v1/realtime")]
fn route_realtime_reads_only(#[case] route: &str) {
    assert!(managed_agent_route_allowed(route, None));
    assert!(managed_agent_route_allowed(route, Some("GET")));
    assert!(!managed_agent_route_allowed(route, Some("HEAD")));
    assert!(!managed_agent_route_allowed(route, Some("POST")));
}

#[rstest]
#[case::chat("/chat/completions")]
#[case::v1_chat("/v1/chat/completions")]
#[case::v1_responses("/v1/responses")]
#[case::openai_responses("/openai/v1/responses")]
#[case::v2_rerank("/v2/rerank")]
#[case::cursor("/cursor/chat/completions")]
#[case::claude_gateway("/claude_code_gateway/v1/messages/count_tokens")]
#[case::ocr("/v1/ocr")]
fn route_model_routes_post_only(#[case] route: &str) {
    assert!(managed_agent_route_allowed(route, None));
    assert!(managed_agent_route_allowed(route, Some("POST")));
    assert!(!managed_agent_route_allowed(route, Some("GET")));
}

#[rstest]
#[case::engines("/engines/gpt-4/chat/completions")]
#[case::deployments("/openai/deployments/my-deployment/embeddings")]
#[case::deployment_images("/openai/deployments/d/images/generations")]
#[case::gemini_count("/v1beta/models/gemini-pro:countTokens")]
#[case::gemini_generate("/v1beta/models/gemini-pro:generateContent")]
#[case::models_stream("/models/foo:streamGenerateContent")]
fn route_model_paths_post_only(#[case] route: &str) {
    assert!(managed_agent_route_allowed(route, None));
    assert!(managed_agent_route_allowed(route, Some("POST")));
    assert!(!managed_agent_route_allowed(route, Some("GET")));
}

#[rstest]
#[case::mcp_root("/mcp")]
#[case::mcp_tools("/mcp/tools")]
#[case::mcp_tools_call("/mcp/tools/call")]
#[case::mcp_rest("/mcp-rest/tools/list")]
#[case::v1_mcp_tools("/v1/mcp/tools")]
#[case::mcp_proxy("/mcp/proxy")]
fn route_mcp_inference_any_method(#[case] route: &str) {
    assert!(managed_agent_route_allowed(route, None));
    assert!(managed_agent_route_allowed(route, Some("GET")));
    assert!(managed_agent_route_allowed(route, Some("POST")));
    assert!(managed_agent_route_allowed(route, Some("DELETE")));
}

#[rstest]
#[case::token("/token")]
#[case::introspect("/introspect")]
fn route_mcp_token_routes_denied(#[case] route: &str) {
    assert!(!managed_agent_route_allowed(route, None));
    assert!(!managed_agent_route_allowed(route, Some("POST")));
}

#[rstest]
#[case::a2a_root("/a2a/agent-1")]
#[case::message_send("/a2a/agent-1/message/send")]
#[case::v1_message_send("/v1/a2a/agent-1/message/send")]
#[case::message_stream("/a2a/agent-1/message/stream")]
#[case::agent_card("/a2a/agent-1/.well-known/agent-card.json")]
fn route_agent_inference_any_method(#[case] route: &str) {
    assert!(managed_agent_route_allowed(route, None));
    assert!(managed_agent_route_allowed(route, Some("GET")));
    assert!(managed_agent_route_allowed(route, Some("DELETE")));
}

#[rstest]
#[case::unknown("/v1/unknown")]
#[case::agent_management("/v1/agents/agent-1")]
#[case::identity_providers("/v1/agents/identity/providers")]
fn route_unknown_routes_denied(#[case] route: &str) {
    assert!(!managed_agent_route_allowed(route, None));
    assert!(!managed_agent_route_allowed(route, Some("POST")));
}

#[rustfmt::skip]
pub(crate) const MCP_INFERENCE_ROUTES: &[&str] = &["/mcp", "/mcp/", "/mcp/sse/", "/mcp/sse/messages", "/mcp/sse/messages/", "/mcp/proxy", "/mcp/{subpath}", "/mcp/tools", "/mcp/tools/list", "/mcp/tools/call", "/mcp-rest/tools/list", "/mcp-rest/tools/call", "/v1/mcp/tools", "/introspect", "/token"];

#[rustfmt::skip]
pub(crate) const MCP_MANAGEMENT_ROUTES: &[&str] = &["/v1/mcp/server", "/v1/mcp/server/{path:path}", "/v1/mcp/sessions"];

#[rustfmt::skip]
pub(crate) const MCP_ROUTES: &[&str] = &["/mcp", "/mcp/", "/mcp/sse/", "/mcp/sse/messages", "/mcp/sse/messages/", "/mcp/proxy", "/mcp/{subpath}", "/mcp/tools", "/mcp/tools/list", "/mcp/tools/call", "/mcp-rest/tools/list", "/mcp-rest/tools/call", "/v1/mcp/tools", "/introspect", "/token", "/v1/mcp/server", "/v1/mcp/server/{path:path}", "/v1/mcp/sessions"];

#[rustfmt::skip]
pub(crate) const AGENT_INFERENCE_ROUTES: &[&str] = &["/agents", "/a2a/{agent_id}", "/a2a/{agent_id}/message/send", "/v1/a2a/{agent_id}/message/send", "/a2a/{agent_id}/message/stream", "/a2a/{agent_id}/.well-known/agent-card.json"];

#[rustfmt::skip]
pub(crate) const AGENT_MANAGEMENT_ROUTES: &[&str] = &["/v1/agents", "/v1/agents/{agent_id}", "/v1/agents/make_public", "/v1/agents/{agent_id}/make_public", "/v1/agents/{agent_id}/kill_switch"];

#[rustfmt::skip]
pub(crate) const AGENT_ROUTES: &[&str] = &["/agents", "/a2a/{agent_id}", "/a2a/{agent_id}/message/send", "/v1/a2a/{agent_id}/message/send", "/a2a/{agent_id}/message/stream", "/a2a/{agent_id}/.well-known/agent-card.json", "/v1/agents", "/v1/agents/{agent_id}", "/v1/agents/make_public", "/v1/agents/{agent_id}/make_public", "/v1/agents/{agent_id}/kill_switch"];

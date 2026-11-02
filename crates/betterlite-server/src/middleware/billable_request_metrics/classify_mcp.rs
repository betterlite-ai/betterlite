mod tests;

use regex::Regex;

const MCP_MANAGEMENT_PREFIX: &str = "/v1/mcp";
const MCP_MANAGEMENT_PREFIX_SLASH: &str = "/v1/mcp/";
// The REST wrapper's tool-call endpoint bills like the /mcp transport; its list/test siblings do not.
const MCP_REST_TOOL_CALL: &str = "/mcp-rest/tools/call";
const MCP_DYNAMIC_TRANSPORT_PATTERN: &str = "^/(?:toolset/)?[^/]+/mcp$";

pub(super) fn classify_mcp_route(path: &str) -> Option<&'static str> {
    if path == MCP_MANAGEMENT_PREFIX || path.starts_with(MCP_MANAGEMENT_PREFIX_SLASH) {
        return None;
    }
    if path == "/mcp" || path.starts_with("/mcp/") {
        return Some("/mcp");
    }
    if path == MCP_REST_TOOL_CALL {
        return Some("/mcp");
    }
    if Regex::new(MCP_DYNAMIC_TRANSPORT_PATTERN).is_ok_and(|pattern| pattern.is_match(path)) {
        return Some("/mcp");
    }
    None
}

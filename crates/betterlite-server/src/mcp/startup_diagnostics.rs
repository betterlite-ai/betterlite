mod tests;

use betterlite_mcp::server_payload::names::validate_tool_name;
use tracing::warn;

pub const SEP_986_URL: &str =
    "https://modelcontextprotocol.io/specification/2025-11-25/server/tools#tool-names";

// LiteLLM's import-time separator probe: the probe carries text on both sides of the separator, and an invalid one only warns
pub fn check_separator(separator: &str) {
    let probe = format!("litellm{separator}probe");
    if !validate_tool_name(&probe).is_valid {
        warn!("MCP_TOOL_PREFIX_SEPARATOR '{separator}' violates SEP-986. See {SEP_986_URL}");
    }
}

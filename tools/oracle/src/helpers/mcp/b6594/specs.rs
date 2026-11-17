use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::error::HelperError;
use serde::Deserialize;

use super::failure;

// Only the fields the server_utils functions read; LiteLLM's MCPServer carries the rest as defaults.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerSpec {
    pub server_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub spec_path: Option<String>,
}

pub(crate) fn server_from(spec: &ServerSpec) -> Result<MCPServer, Box<HelperError>> {
    let raw = serde_json::json!({
        "server_id": spec.server_id.clone(),
        "name": spec.server_id.clone(),
        "transport": "http",
        "alias": spec.alias.clone(),
        "server_name": spec.server_name.clone(),
        "spec_path": spec.spec_path.clone(),
    });
    serde_json::from_value(raw).map_err(|error| failure("InvalidInput", error.to_string()))
}

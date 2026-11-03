use std::collections::BTreeMap;

use betterlite_server::mcp::catalog_guard::guard::GuardrailFailure;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    StripPrefix {
        name: String,
        #[serde(default)]
        server: Option<ServerInput>,
    },
    SanitizeHeaders {
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        upstream_credential_names: Vec<String>,
        #[serde(default)]
        identity_header_names: Vec<String>,
        #[serde(default = "default_auth_header")]
        client_side_auth_header_name: String,
        #[serde(default)]
        custom_key_header_name: Option<String>,
    },
    Pin {
        tools: Vec<ToolInput>,
        pinned: BTreeMap<String, PinnedInput>,
    },
    Override {
        tools: Vec<ToolInput>,
        overrides: BTreeMap<String, String>,
        #[serde(default)]
        prefix: Option<String>,
    },
    #[serde(alias = "batch")]
    Scan {
        tools: Vec<ToolInput>,
        #[serde(default)]
        outcomes: BTreeMap<String, HookOutcome>,
        #[serde(default)]
        upstream_credential_names: Vec<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        batch_report: bool,
    },
    Error {
        failure: GuardrailFailure,
    },
}

#[derive(Deserialize)]
pub(crate) struct ToolInput {
    pub(super) name: String,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) title: Option<String>,
    #[serde(default, alias = "inputSchema")]
    pub(super) input_schema: BTreeMap<String, Value>,
    #[serde(default, alias = "outputSchema")]
    pub(super) output_schema: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    pub(super) execution: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    pub(super) annotations: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    pub(super) icons: Option<Vec<Value>>,
    #[serde(default, rename = "_meta")]
    pub(super) meta: Option<BTreeMap<String, Value>>,
}

#[derive(Deserialize)]
pub(crate) struct PinnedInput {
    #[serde(default)]
    pub(super) description: String,
    #[serde(default, alias = "inputSchema")]
    pub(super) input_schema: BTreeMap<String, Value>,
}

#[derive(Deserialize, Default)]
pub(crate) struct ServerInput {
    #[serde(default)]
    pub(super) server_id: String,
    #[serde(default = "default_server_name")]
    pub(super) name: String,
    #[serde(default)]
    pub(super) alias: Option<String>,
    #[serde(default)]
    pub(super) server_name: Option<String>,
    #[serde(default)]
    pub(super) short_prefix: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
pub(crate) struct HookOutcome {
    #[serde(default)]
    pub(super) guarded: Option<GuardedInput>,
    #[serde(default)]
    pub(super) failure: Option<GuardrailFailure>,
}

#[derive(Clone, Deserialize, Default)]
pub(crate) struct GuardedInput {
    #[serde(default, rename = "mcp_tool_description")]
    pub(super) description: Option<Value>,
    #[serde(default, rename = "mcp_tool_description_present")]
    pub(super) description_present: bool,
    #[serde(default, rename = "mcp_input_schema")]
    pub(super) schema: Option<Value>,
    #[serde(default, rename = "mcp_input_schema_present")]
    pub(super) schema_present: bool,
}

fn default_auth_header() -> String {
    "x-mcp-auth".to_owned()
}

fn default_server_name() -> String {
    "server".to_owned()
}

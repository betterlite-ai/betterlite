use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use betterlite_mcp::server_utils::allowlist::{
    allowlist_enforced, server_applies_tool_allowlist, tool_result_error_message,
};
use betterlite_mcp::server_utils::headers::merge_mcp_headers;
use betterlite_mcp::server_utils::synthetic::build_synthetic_mcp_request;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{failure, stored_json};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum WireInput {
    Allowlist(AllowlistInput),
    ToolResultError(ToolResultInput),
    MergeHeaders(MergeHeadersInput),
    Synthetic(SyntheticInput),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AllowlistInput {
    #[serde(default)]
    pub mcp_info: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolResultInput {
    pub result: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeHeadersInput {
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub static_headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SyntheticInput {
    pub path: String,
    #[serde(default)]
    pub raw_headers: Option<Vec<(String, String)>>,
    #[serde(default)]
    pub client_ip: Option<String>,
    pub extra_excluded: Vec<String>,
}

pub(crate) fn run(input: &WireInput) -> HelperResult {
    match input {
        WireInput::Allowlist(fields) => allowlist(fields),
        WireInput::ToolResultError(fields) => Ok(json!(tool_result_error_message(&stored_json(
            &fields.result
        )?))),
        WireInput::MergeHeaders(fields) => Ok(json!(merge_mcp_headers(
            fields.extra_headers.as_ref(),
            fields.static_headers.as_ref()
        ))),
        WireInput::Synthetic(fields) => synthetic(fields),
    }
}

fn allowlist(fields: &AllowlistInput) -> HelperResult {
    let info = fields
        .mcp_info
        .as_ref()
        .map(|metadata| {
            let stored: BTreeMap<String, JsonValue> = metadata
                .iter()
                .map(|(name, value)| Ok((name.clone(), stored_json(value)?)))
                .collect::<Result<_, Box<HelperError>>>()?;
            MCPInfo::try_from(stored).map_err(|error| failure("InvalidInput", error.to_string()))
        })
        .transpose()?;
    let enforced = allowlist_enforced(info.as_ref());
    let applies = server_applies_tool_allowlist(info.as_ref(), fields.allowed_tools.as_deref());
    Ok(json!({"enforced": enforced, "applies": applies}))
}

fn synthetic(fields: &SyntheticInput) -> HelperResult {
    let request = build_synthetic_mcp_request(
        &fields.path,
        fields.raw_headers.as_deref(),
        fields.client_ip.as_deref(),
        &fields.extra_excluded,
    );
    serde_json::to_value(&request).map_err(|error| failure("Serialization", error.to_string()))
}

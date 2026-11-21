pub mod error;
pub mod from_llm;
pub mod kwargs;
pub mod to_llm;

mod tests;

use std::collections::BTreeMap;

use betterlite_models::decisions::{HiddenParams, Json};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnNull, serde_as};

use crate::mcp::hook_payloads::error::HookPayloadError;

pub type ToolArguments = IndexMap<String, Json>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpToolCall<'call> {
    pub tool_name: &'call str,
    pub arguments: &'call ToolArguments,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPreCallRequest {
    pub tool_name: String,
    pub arguments: ToolArguments,
    pub server_name: Option<String>,
    pub user_api_key_auth: IndexMap<String, Json>,
    pub hidden_params: HiddenParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpPreCallResponse {
    pub should_proceed: bool,
    pub modified_arguments: Option<ToolArguments>,
    pub error_message: Option<String>,
    pub hidden_params: HiddenParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpDuringCallResponse {
    pub should_continue: bool,
    pub error_message: Option<String>,
    pub hidden_params: HiddenParams,
}

#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct GuardrailContext {
    pub model: Option<String>,
    #[serde_as(as = "DefaultOnNull")]
    pub metadata: IndexMap<String, Json>,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpHookKwargs {
    #[serde_as(as = "DefaultOnNull")]
    pub guardrail_context: GuardrailContext,
    #[serde_as(as = "DefaultOnNull")]
    pub name: String,
    #[serde_as(as = "DefaultOnNull")]
    pub arguments: ToolArguments,
    pub server_name: Option<String>,
    pub mcp_rate_limit_server_name: Option<String>,
    // the auth record as model_dump writes it, so its key, team and project metadata are read by name
    pub user_api_key_auth: Option<IndexMap<String, Json>>,
    pub user_api_key_user_id: Option<String>,
    pub user_api_key_team_id: Option<String>,
    pub user_api_key_end_user_id: Option<String>,
    pub user_api_key_hash: Option<String>,
    pub user_api_key_request_route: Option<String>,
    pub incoming_bearer_token: Option<SecretString>,
    #[serde_as(as = "DefaultOnNull")]
    pub headers: BTreeMap<String, String>,
    pub model: Option<String>,
    pub mcp_tool_description: Option<String>,
    pub mcp_input_schema: Option<IndexMap<String, Json>>,
    pub extra_headers: Option<BTreeMap<String, String>>,
}

// the request data a guardrail hook hands back carries every synthetic field; only these three are read
#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct GuardrailHookData {
    #[serde_as(as = "DefaultOnNull")]
    pub messages: Vec<HookMessage>,
    pub modified_arguments: Option<ToolArguments>,
    pub extra_headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct HookMessage {
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardrailOutcome {
    Raised(String),
    Text(String),
    Data(Box<GuardrailHookData>),
}

// LiteLLM's guardrails read this text, and the answer counts as changed when it no longer matches it
fn tool_call_text(
    call: McpToolCall<'_>,
    description: Option<&str>,
) -> Result<String, HookPayloadError> {
    let arguments = serde_json::to_string(call.arguments).map_err(HookPayloadError::Arguments)?;
    let description_line =
        description.map_or_else(String::new, |text| format!("\nDescription: {text}"));
    Ok(format!(
        "Tool: {}{description_line}\nArguments: {arguments}",
        call.tool_name
    ))
}

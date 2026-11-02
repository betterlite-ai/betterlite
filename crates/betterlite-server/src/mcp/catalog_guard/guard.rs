use std::collections::BTreeMap;
use std::future::Future;

use betterlite_db::mcp_stored_contracts::JsonValue;

use super::types::MCPServer;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallerContext {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub end_user_id: Option<String>,
    pub api_key_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailFailure {
    pub exception_type: String,
    pub exception_message: String,
    pub detail_error: Option<String>,
}

pub struct ScanContext<'context> {
    pub server: &'context MCPServer,
    pub caller: Option<&'context CallerContext>,
    pub headers: &'context BTreeMap<String, String>,
    pub identity_header_names: &'context [String],
    pub custom_key_header_name: Option<&'context str>,
    pub client_side_auth_header_name: &'context str,
}

pub struct GuardrailCallContext<'context> {
    pub call_type: &'context str,
    pub guardrails_only: bool,
    pub tool_name: &'context str,
    pub server_name: &'context str,
    pub rate_limit_server_name: &'context str,
    pub caller: Option<&'context CallerContext>,
    pub headers: &'context BTreeMap<String, String>,
    pub description: &'context str,
    pub input_schema: &'context BTreeMap<String, JsonValue>,
}

pub trait GuardrailHook {
    fn pre_call_hook(
        &self,
        context: GuardrailCallContext<'_>,
    ) -> impl Future<Output = Result<Option<BTreeMap<String, JsonValue>>, GuardrailFailure>> + Send;
}

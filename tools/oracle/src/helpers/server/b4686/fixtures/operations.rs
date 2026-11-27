use std::sync::PoisonError;

use async_trait::async_trait;
use betterlite_mcp::wire_compat::tool_outcome::{TextResult, ToolOutcome};
use betterlite_server::mcp::catalog_guard::types::MCPTool;
use betterlite_server::mcp::search_handlers::ports::{
    McpDenial, McpOperations, SearchPortError, ToolExecution,
};
use betterlite_server::mcp::search_handlers::types::{CallerScope, McpRequestScope};
use serde_json::json;

use super::fakes::Fakes;
use super::records::{
    allowed_call_json, caller_json, execution_scope_json, guardrail_json, scope_json,
};

impl Fakes<'_> {
    fn allowed_response(&self, caller: &CallerScope, scope: &McpRequestScope) -> Vec<String> {
        let record = allowed_call_json(caller, scope);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .allowed_calls
            .push(record);
        self.input.allowed_servers.clone()
    }
}

#[async_trait]
impl McpOperations for Fakes<'_> {
    async fn _list_mcp_tools(
        &self,
        caller: &CallerScope,
        scope: &McpRequestScope,
        proxy_mode: bool,
    ) -> Result<Vec<MCPTool>, SearchPortError> {
        let record = json!({
            "proxy_mode": proxy_mode,
            "caller": caller_json(caller),
            "scope": scope_json(scope),
        });
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .list_calls
            .push(record);
        Ok(self.input.listed_tools())
    }

    async fn _get_allowed_mcp_servers(
        &self,
        caller: &CallerScope,
        scope: &McpRequestScope,
    ) -> Result<Vec<String>, SearchPortError> {
        Ok(self.allowed_response(caller, scope))
    }

    async fn raise_denied_scoped_mcp_access(
        &self,
        caller: &CallerScope,
        scope: &McpRequestScope,
        requested_names: &[String],
    ) -> McpDenial {
        let record = json!({
            "requested": requested_names,
            "caller": caller_json(caller),
            "client_ip": scope.client_ip,
        });
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .denials
            .push(record);
        McpDenial {
            detail: self.input.denial_detail.clone().unwrap_or_default(),
        }
    }

    async fn execute_mcp_tool(
        &self,
        call: ToolExecution<'_>,
    ) -> Result<ToolOutcome, SearchPortError> {
        let arguments = serde_json::to_value(call.arguments).map_err(|error| SearchPortError {
            detail: error.to_string(),
        })?;
        let record = json!({
            "tool_name": call.tool_name,
            "arguments": arguments,
            "allowed_servers": call.allowed_servers,
            "requested_server_id": call.requested_server_id,
            "guardrail_context": guardrail_json(call.guardrail_context)?,
            "caller": caller_json(call.caller),
            "scope": execution_scope_json(call.scope),
        });
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .executions
            .push(record);
        let name = call.tool_name;
        Ok(ToolOutcome::Text(TextResult {
            text: format!("executed {name}"),
        }))
    }
}

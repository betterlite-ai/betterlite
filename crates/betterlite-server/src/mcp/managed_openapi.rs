mod tests;

use betterlite_mcp::client::tools::result::{CallToolResult, ContentBlock, TextContent};
use betterlite_mcp::openapi_tools::output::OpenApiToolOutput;
use betterlite_mcp::openapi_tools::request_headers::OpenApiCallContext;
use betterlite_mcp::openapi_tools::tool::OpenApiToolError;
use betterlite_mcp::server_record::MCPServer;
use betterlite_mcp::server_utils::names::add_server_prefix_to_name;
use betterlite_mcp::server_utils::prefixes::{PrefixPolicy, get_server_prefix};
use betterlite_mcp::tool_registry::handler::ToolHandlerError;
use betterlite_mcp::tool_registry::registry::ToolRegistry;
use betterlite_mcp::wire_compat::result_conversion::complete_call_tool_result;
use betterlite_mcp::wire_compat::tool_outcome::{JsonResult, TextResult, ToolOutcome, WireCompat};
use betterlite_prompts::arguments::json::Object;
use thiserror::Error;
use tracing::error;

#[derive(Clone, Copy)]
pub struct ManagedOpenApiCall<'call> {
    pub server: &'call MCPServer,
    pub tool_name: &'call str,
    pub arguments: &'call Object,
    pub context: &'call OpenApiCallContext,
    pub prefix_policy: &'call PrefixPolicy,
}

#[derive(Debug, Error)]
pub enum ManagedOpenApiError {
    #[error("upstream server {server:?} answered with status {status} and a challenge")]
    UpstreamAuth {
        status: u16,
        www_authenticate: Option<String>,
        server: String,
    },
}

// LiteLLM's _call_openapi_tool_handler: rebuild the registration key, call the handler, convert its outcome
pub async fn call(
    registry: &ToolRegistry,
    request: ManagedOpenApiCall<'_>,
    wire_compat: WireCompat,
) -> Result<CallToolResult, ManagedOpenApiError> {
    let prefix = get_server_prefix(request.server, request.prefix_policy);
    let registry_key =
        add_server_prefix_to_name(request.tool_name, &prefix, &request.prefix_policy.separator);
    // the entry is cloned out of the registry before the handler is awaited, so the lock is never held across it
    let Some(tool) = registry.get_tool(&registry_key) else {
        return Ok(missing_registration(request.tool_name));
    };
    match tool.handler.call(request.arguments, request.context).await {
        Ok(output) => Ok(complete_call_tool_result(tool_outcome(output), wire_compat)),
        Err(ToolHandlerError::OpenApi(OpenApiToolError::UpstreamAuth {
            status,
            www_authenticate,
            server,
        })) => Err(ManagedOpenApiError::UpstreamAuth {
            status,
            www_authenticate,
            server,
        }),
        Err(failure) => Ok(failed_call(request.tool_name, &failure)),
    }
}

fn missing_registration(tool_name: &str) -> CallToolResult {
    let message = format!("OpenAPI tool {tool_name} not found in registry");
    error!("{message}");
    error_result(message)
}

fn failed_call(tool_name: &str, failure: &ToolHandlerError) -> CallToolResult {
    let message = format!("Error calling OpenAPI tool {tool_name}: {failure}");
    error!("{message}");
    error_result(message)
}

fn error_result(message: String) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text(TextContent {
            text: message,
            extra: Object::new(),
        })],
        is_error: true,
        structured_content: None,
        meta: None,
    }
}

fn tool_outcome(output: OpenApiToolOutput) -> ToolOutcome {
    match output {
        OpenApiToolOutput::Text(text) => ToolOutcome::Text(TextResult { text }),
        OpenApiToolOutput::Json {
            value,
            original_text,
        } => ToolOutcome::Json(JsonResult {
            value,
            original_text,
        }),
    }
}

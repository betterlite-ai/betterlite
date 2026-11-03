pub mod description_overrides;
pub mod guard;
pub mod headers;
pub mod pin;
pub mod prefix;
mod tests;
pub mod types;

use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;
use futures_util::future::join_all;
use serde::Serialize;

use self::guard::{GuardrailCallContext, GuardrailCallType, GuardrailFailure, ScanContext};
use self::headers::{HeaderSanitizeContext, logging_safe_mcp_headers};
use self::types::{MCPServer, MCPTool};

const CATALOG_SCAN_BATCH_SIZE: usize = 8;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CatalogAlert {
    pub signature: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BlockedTool {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ToolDescriptionScan {
    pub served: Vec<MCPTool>,
    pub blocked: Vec<BlockedTool>,
}

impl ToolDescriptionScan {
    #[must_use]
    pub fn alert(&self, server: &MCPServer) -> Option<CatalogAlert> {
        if self.blocked.is_empty() {
            return None;
        }
        let mut names: Vec<&str> = self.blocked.iter().map(|tool| tool.name.as_str()).collect();
        names.sort_unstable();
        let lines: Vec<String> = self
            .blocked
            .iter()
            .map(|tool| format!("- `{}`: {}", tool.name, tool.reason))
            .collect();
        Some(CatalogAlert {
            signature: names.join(","),
            message: format!(
                "MCP server `{}`: {} tool description(s) blocked by a guardrail and hidden from tools/list\n{}",
                server.name,
                self.blocked.len(),
                lines.join("\n"),
            ),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedCatalogDrift {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

impl PinnedCatalogDrift {
    #[must_use]
    pub fn alert(&self, server: &MCPServer) -> Option<CatalogAlert> {
        let parts = drift_parts(self);
        (!parts.is_empty()).then(|| CatalogAlert {
            signature: parts.join("|"),
            message: format!(
                "MCP server `{}`: upstream tool list drifted from the pinned catalog; serving the pinned tools and descriptions until an admin re-pins the server\n{}",
                server.name,
                parts.join("\n"),
            ),
        })
    }
}

fn drift_parts(drift: &PinnedCatalogDrift) -> Vec<String> {
    let labeled = [
        ("added", &drift.added),
        ("removed", &drift.removed),
        ("changed", &drift.changed),
    ];
    labeled
        .into_iter()
        .filter(|(_, names)| !names.is_empty())
        .map(|(label, names)| {
            let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
            format!("{label}: {}", quoted.join(", "))
        })
        .collect()
}

pub async fn scan_tool_descriptions<H>(
    tools: &[MCPTool],
    context: &ScanContext<'_>,
    hook: &H,
) -> ToolDescriptionScan
where
    H: guard::GuardrailHook + Sync,
{
    let mut served = Vec::new();
    let mut blocked = Vec::new();
    for batch in tools.chunks(CATALOG_SCAN_BATCH_SIZE) {
        let outcomes = join_all(batch.iter().map(|tool| scan_tool(tool, context, hook))).await;
        for outcome in outcomes {
            match outcome {
                ScanOutcome::Served(tool) => served.push(tool),
                ScanOutcome::Blocked(tool) => blocked.push(tool),
            }
        }
    }
    ToolDescriptionScan { served, blocked }
}

enum ScanOutcome {
    Served(MCPTool),
    Blocked(BlockedTool),
}

// Upstream helper spellings are fn _has_scannable_text and fn _block_reason; local names avoid Rust underscore-item lints.
fn has_scannable_text(tool: &MCPTool) -> bool {
    if tool
        .description
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return true;
    }
    !tool.input_schema.is_empty()
}

async fn scan_tool<H>(tool: &MCPTool, context: &ScanContext<'_>, hook: &H) -> ScanOutcome
where
    H: guard::GuardrailHook + Sync,
{
    if !has_scannable_text(tool) {
        return ScanOutcome::Served(tool.clone());
    }
    match guarded_catalog_entry(tool, context, hook).await {
        Ok(Some(guarded)) => match masked_tool(tool, &guarded) {
            Ok(masked) => ScanOutcome::Served(masked),
            Err(failure) => ScanOutcome::Blocked(BlockedTool {
                name: tool.name.clone(),
                reason: block_reason(&failure),
            }),
        },
        Ok(None) => ScanOutcome::Served(tool.clone()),
        Err(failure) => ScanOutcome::Blocked(BlockedTool {
            name: tool.name.clone(),
            reason: block_reason(&failure),
        }),
    }
}

async fn guarded_catalog_entry<H>(
    tool: &MCPTool,
    context: &ScanContext<'_>,
    hook: &H,
) -> Result<Option<BTreeMap<String, JsonValue>>, GuardrailFailure>
where
    H: guard::GuardrailHook + Sync,
{
    let header_context = HeaderSanitizeContext {
        upstream_credential_names: context.upstream_credential_names,
        identity_header_names: context.identity_header_names,
        custom_key_header_name: context.custom_key_header_name,
        client_side_auth_header_name: context.client_side_auth_header_name,
    };
    let safe_headers = logging_safe_mcp_headers(context.headers, &header_context);
    let rate_limit_server_name = context.server.rate_limit_name().to_owned();
    let description = tool.description.clone().unwrap_or_default();
    let call_context = GuardrailCallContext {
        call_type: GuardrailCallType::ListMcpTools,
        guardrails_only: true,
        tool_name: &tool.name,
        server_name: &context.server.name,
        rate_limit_server_name: &rate_limit_server_name,
        caller: context.caller,
        headers: &safe_headers,
        description: &description,
        input_schema: &tool.input_schema,
    };
    hook.pre_call_hook(call_context).await
}

#[must_use]
pub fn block_reason(failure: &GuardrailFailure) -> String {
    match failure
        .detail_error
        .as_deref()
        .filter(|error| !error.is_empty())
    {
        Some(error) => error.to_owned(),
        None if !failure.exception_message.is_empty() => {
            format!("{}: {}", failure.exception_type, failure.exception_message)
        }
        None => failure.exception_type.clone(),
    }
}

fn masked_tool(
    tool: &MCPTool,
    guarded: &BTreeMap<String, JsonValue>,
) -> Result<MCPTool, GuardrailFailure> {
    let description = match guarded.get("mcp_tool_description") {
        Some(JsonValue::String(value)) => Some(value.clone()),
        Some(JsonValue::Null) => None,
        Some(value) => return Err(invalid_guarded_value("mcp_tool_description", value)),
        None => tool.description.clone(),
    };
    let input_schema = match guarded.get("mcp_input_schema") {
        Some(JsonValue::Object(value)) => value.clone(),
        Some(value) => return Err(invalid_guarded_value("mcp_input_schema", value)),
        None => tool.input_schema.clone(),
    };
    if description == tool.description && input_schema == tool.input_schema {
        Ok(tool.clone())
    } else {
        Ok(MCPTool {
            description,
            input_schema,
            ..tool.clone()
        })
    }
}

fn invalid_guarded_value(field: &str, value: &JsonValue) -> GuardrailFailure {
    GuardrailFailure {
        exception_type: "ValidationError".to_owned(),
        exception_message: format!("{field} must have a valid MCP value, got {value:?}"),
        detail_error: None,
    }
}

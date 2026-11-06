use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;
use serde_json::value::{RawValue, to_raw_value};

use super::error::UnifiedError;
use super::request_data::RequestData;
use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit};
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_detail::{GuardrailHttpError, build_error_detail};
use crate::panw_prisma_airs::error_handling::{handle_api_error_with_logging, record_scan_id};
use crate::panw_prisma_airs::metadata::ScanMetadata;
use crate::panw_prisma_airs::payload::ScanRequest;
use crate::panw_prisma_airs::scan::call_panw_api;
use crate::panw_prisma_airs::verdict::{Action, ScanVerdict};
use crate::prisma_airs_masking::get_masked_text;
use crate::prisma_airs_masking::mcp::{McpRequest, McpServer, get_mcp_server_name};

#[derive(Debug, Clone, Copy)]
pub struct McpScan<'scan> {
    pub direction: Direction,
    pub metadata: &'scan ScanMetadata,
    pub call_id: &'scan str,
    pub start_time: SystemTime,
    pub servers_by_id: &'scan BTreeMap<String, McpServer>,
    pub names_by_tool: &'scan BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct ToolEvent<'event> {
    metadata: ToolMetadata<'event>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<&'event str>,
}

#[derive(Debug, Serialize)]
struct ToolMetadata<'event> {
    ecosystem: &'static str,
    method: &'static str,
    server_name: &'event str,
    tool_invoked: &'event str,
}

// Mirrors the MCP REST branch of LiteLLM's fn apply_guardrail: one tool_event scan, then masking on the request fields.
pub async fn scan_mcp_tool(
    guardrail: &PanwPrismaAirs,
    request_data: &mut RequestData,
    scan: &McpScan<'_>,
    audit: &mut GuardrailAudit,
) -> Result<(), UnifiedError> {
    if scan.direction != Direction::Request {
        return Ok(());
    }
    let Some(tool_name) = request_data.resolved_tool_name().map(str::to_owned) else {
        return Ok(());
    };
    let event = tool_event(request_data, &tool_name, scan)?;
    let verdict = call_panw_api(
        guardrail,
        ScanRequest {
            content: "",
            direction: Direction::Request,
            metadata: Some(scan.metadata),
            call_id: Some(scan.call_id),
            tool_event: Some(&event),
        },
    )
    .await;
    if verdict.failure.is_some() {
        handle_api_error_with_logging(
            guardrail,
            &verdict,
            audit,
            scan.start_time,
            Direction::Request,
        )?;
        return Ok(());
    }
    record_scan_id(guardrail, audit, &verdict, Direction::Request);
    apply_verdict(guardrail, request_data, &verdict)
}

fn tool_event(
    request_data: &RequestData,
    tool_name: &str,
    scan: &McpScan<'_>,
) -> Result<Box<RawValue>, UnifiedError> {
    let server_name = mcp_server_name(request_data, tool_name, scan);
    let input = mcp_input(request_data).map_err(UnifiedError::Encode)?;
    let event = ToolEvent {
        metadata: ToolMetadata {
            ecosystem: "mcp",
            method: "tools/call",
            server_name: &server_name,
            tool_invoked: tool_name,
        },
        input: input.as_deref(),
    };
    to_raw_value(&event).map_err(UnifiedError::Encode)
}

fn mcp_server_name(request_data: &RequestData, tool_name: &str, scan: &McpScan<'_>) -> String {
    let request = McpRequest {
        mcp_server_name: request_data.mcp_server_name.clone(),
        server_name: request_data.server_name.clone(),
        server_id: request_data.server_id.clone(),
        arguments: None,
        mcp_arguments: None,
    };
    get_mcp_server_name(&request, scan.servers_by_id, scan.names_by_tool, tool_name)
}

fn mcp_input(request_data: &RequestData) -> Result<Option<String>, serde_json::Error> {
    let Some(value) = selected_arguments(request_data) else {
        return Ok(None);
    };
    let serialized = match value {
        GuardrailValue::Text(text) => text.clone(),
        other @ (GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_)) => serde_json::to_string(other)?,
    };
    Ok((!serialized.trim().is_empty()).then_some(serialized))
}

// mcp_arguments wins over arguments; a present null in either falls through like Python's None.
fn selected_arguments(request_data: &RequestData) -> Option<&GuardrailValue> {
    request_data
        .mcp_arguments
        .as_ref()
        .filter(|value| !matches!(value, GuardrailValue::Null))
        .or_else(|| {
            request_data
                .arguments
                .as_ref()
                .filter(|value| !matches!(value, GuardrailValue::Null))
        })
}

fn apply_verdict(
    guardrail: &PanwPrismaAirs,
    request_data: &mut RequestData,
    verdict: &ScanVerdict,
) -> Result<(), UnifiedError> {
    let masked_text = get_masked_text(&verdict.masks, false).filter(|masked| !masked.is_empty());
    let original = selected_arguments(request_data).cloned();
    match verdict.action {
        Action::Allow => {
            if let Some(masked) = masked_text {
                apply_mcp_masking(request_data, original.as_ref(), masked, false)?;
            }
        }
        Action::Block
            if masked_text.is_some() && guardrail.settings.masking.content.request_content =>
        {
            apply_mcp_masking(
                request_data,
                original.as_ref(),
                masked_text.unwrap_or_default(),
                true,
            )?;
        }
        Action::Block => {
            return Err(UnifiedError::Scan(GuardrailHttpError {
                status: 400,
                detail: Box::new(build_error_detail(guardrail, verdict, Direction::Request)),
            }));
        }
    }
    Ok(())
}

fn apply_mcp_masking(
    request_data: &mut RequestData,
    original: Option<&GuardrailValue>,
    masked_text: &str,
    is_blocked: bool,
) -> Result<(), UnifiedError> {
    if request_data.arguments.is_none() && request_data.mcp_arguments.is_none() {
        return Err(UnifiedError::McpArgumentsMissing);
    }
    // Structured arguments keep their type through a JSON parse; plain text is stored as given.
    let masked = match original {
        Some(GuardrailValue::Object(_) | GuardrailValue::Array(_)) => {
            serde_json::from_str::<GuardrailValue>(masked_text)
                .map_err(UnifiedError::McpMaskedJson)?
        }
        _ => GuardrailValue::Text(masked_text.to_owned()),
    };
    if request_data.arguments.is_some() {
        request_data.arguments = Some(masked.clone());
    }
    if request_data.mcp_arguments.is_some() {
        request_data.mcp_arguments = Some(masked);
    }
    if is_blocked {
        tracing::warn!(
            "PANW Prisma AIRS: MCP request blocked but masked instead (mask_request_content=True)"
        );
    } else {
        tracing::info!("PANW Prisma AIRS: MCP request allowed with PII masking applied");
    }
    Ok(())
}

mod tests;

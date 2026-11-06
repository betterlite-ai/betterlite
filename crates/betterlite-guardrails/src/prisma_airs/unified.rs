use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use uuid::Uuid;

use super::error::UnifiedError;
use super::inputs::Inputs;
use super::request_data::{LoggingObject, RequestData};
use super::unified_call_id::resolve_call_id;
use super::unified_masking::masked_or_blocked_text;
use super::unified_mcp_tool::{McpScan, scan_mcp_tool};
use super::unified_text_selection::scannable_text_indices;
use super::unified_tool_call::{ToolCallScan, scan_tool_calls};
use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit};
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_handling::{handle_api_error_with_logging, record_scan_id};
use crate::panw_prisma_airs::metadata::ScanMetadata;
use crate::panw_prisma_airs::payload::ScanRequest;
use crate::panw_prisma_airs::scan::call_panw_api;
use crate::prisma_airs_masking::mcp::McpServer;

pub struct ScanContext<'state> {
    pub request_data: &'state mut RequestData,
    pub input_type: Direction,
    pub logging_obj: Option<&'state LoggingObject>,
    pub servers_by_id: &'state BTreeMap<String, McpServer>,
    pub names_by_tool: &'state BTreeMap<String, String>,
    pub new_id: fn() -> Uuid,
}

#[derive(Debug, Clone, Copy)]
struct TextScan<'scan> {
    direction: Direction,
    metadata: &'scan ScanMetadata,
    call_id: &'scan str,
    start_time: SystemTime,
    scannable: Option<&'scan BTreeSet<usize>>,
}

// Mirrors LiteLLM's unified fn apply_guardrail; the texts come back rewritten and tool calls are masked in place.
pub async fn apply_guardrail(
    guardrail: &PanwPrismaAirs,
    inputs: &mut Inputs,
    context: &mut ScanContext<'_>,
    audit: &mut GuardrailAudit,
) -> Result<(), UnifiedError> {
    let call_id = prepare_request(context, inputs);
    let metadata = context.request_data.scan_metadata();
    let start_time = (guardrail.now)();
    let scannable = scannable_text_indices(
        guardrail,
        inputs,
        context.request_data,
        context.logging_obj,
        context.input_type,
    );
    let text_scan = TextScan {
        direction: context.input_type,
        metadata: &metadata,
        call_id: &call_id,
        start_time,
        scannable: scannable.as_ref(),
    };
    let new_texts = scan_texts(guardrail, &inputs.texts, &text_scan, audit).await?;
    let tool_scan = ToolCallScan {
        direction: context.input_type,
        metadata: &metadata,
        call_id: &call_id,
        start_time,
    };
    scan_tool_calls(guardrail, inputs, &tool_scan, audit).await?;
    let mcp_scan = McpScan {
        direction: context.input_type,
        metadata: &metadata,
        call_id: &call_id,
        start_time,
        servers_by_id: context.servers_by_id,
        names_by_tool: context.names_by_tool,
    };
    scan_mcp_tool(guardrail, context.request_data, &mcp_scan, audit).await?;
    inputs.texts = new_texts;
    audit.add_applied_guardrail(&guardrail.settings.guardrail_name);
    Ok(())
}

fn prepare_request(context: &mut ScanContext<'_>, inputs: &Inputs) -> String {
    let call_id = resolve_call_id(
        context.request_data,
        context.logging_obj,
        context.input_type,
        context.new_id,
    );
    context
        .request_data
        .fill_model(inputs.model.as_deref(), context.logging_obj);
    context
        .request_data
        .merge_logging_metadata(context.logging_obj);
    call_id
}

async fn scan_texts(
    guardrail: &PanwPrismaAirs,
    texts: &[String],
    scan: &TextScan<'_>,
    audit: &mut GuardrailAudit,
) -> Result<Vec<String>, UnifiedError> {
    let mut scanned = Vec::with_capacity(texts.len());
    for (index, text) in texts.iter().enumerate() {
        scanned.push(scan_text(guardrail, index, text, scan, audit).await?);
    }
    Ok(scanned)
}

async fn scan_text(
    guardrail: &PanwPrismaAirs,
    index: usize,
    text: &str,
    scan: &TextScan<'_>,
    audit: &mut GuardrailAudit,
) -> Result<String, UnifiedError> {
    if text.trim().is_empty() {
        return Ok(text.to_owned());
    }
    if scan
        .scannable
        .is_some_and(|indices| !indices.contains(&index))
    {
        return Ok(text.to_owned());
    }
    let verdict = call_panw_api(
        guardrail,
        ScanRequest {
            content: text,
            direction: scan.direction,
            metadata: Some(scan.metadata),
            call_id: Some(scan.call_id),
            tool_event: None,
        },
    )
    .await;
    if verdict.failure.is_some() {
        handle_api_error_with_logging(guardrail, &verdict, audit, scan.start_time, scan.direction)?;
        return Ok(text.to_owned());
    }
    record_scan_id(guardrail, audit, &verdict, scan.direction);
    Ok(masked_or_blocked_text(
        guardrail,
        &verdict,
        text,
        scan.direction,
    )?)
}

mod tests;

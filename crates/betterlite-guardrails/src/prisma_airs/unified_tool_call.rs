use std::time::SystemTime;

use super::inputs::Inputs;
use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit};
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_detail::GuardrailHttpError;
use crate::panw_prisma_airs::metadata::ScanMetadata;
use crate::panw_prisma_airs::tool_scan::{ToolScan, scan_tool_calls_for_guardrail};

#[derive(Debug, Clone, Copy)]
pub struct ToolCallScan<'scan> {
    pub direction: Direction,
    pub metadata: &'scan ScanMetadata,
    pub call_id: &'scan str,
    pub start_time: SystemTime,
}

// Mirrors the tool_calls phase of LiteLLM's fn apply_guardrail; every call is scanned on its own and masked in place.
pub async fn scan_tool_calls(
    guardrail: &PanwPrismaAirs,
    inputs: &mut Inputs,
    scan: &ToolCallScan<'_>,
    audit: &mut GuardrailAudit,
) -> Result<(), GuardrailHttpError> {
    if inputs.tool_calls.is_empty() {
        return Ok(());
    }
    let tool_scan = ToolScan {
        direction: scan.direction,
        metadata: scan.metadata,
        call_id: Some(scan.call_id),
        start_time: scan.start_time,
    };
    scan_tool_calls_for_guardrail(guardrail, &mut inputs.tool_calls, &tool_scan, audit).await
}

mod tests;

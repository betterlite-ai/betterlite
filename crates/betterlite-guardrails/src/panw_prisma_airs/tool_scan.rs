use std::time::SystemTime;

use serde::Deserialize;
use serde_json::value::RawValue;

use super::audit::{Direction, GuardrailAudit};
use super::construct::PanwPrismaAirs;
use super::error_detail::{GuardrailHttpError, build_error_detail};
use super::error_handling::{handle_api_error_with_logging, record_scan_id};
use super::metadata::ScanMetadata;
use super::payload::ScanRequest;
use super::scan::call_panw_api;
use super::verdict::{Action, ScanVerdict};
use crate::prisma_airs_masking::get_masked_text;

// The function slice of a tool call; custom tool calls carry no function and are not scanned.
#[derive(Debug, Deserialize)]
pub struct ToolCall {
    pub function: Option<ToolFunction>,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "ToolFunctionWire")]
pub struct ToolFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ToolFunctionWire {
    name: Option<Box<RawValue>>,
    arguments: Option<Box<RawValue>>,
}

impl TryFrom<ToolFunctionWire> for ToolFunction {
    type Error = serde_json::Error;

    fn try_from(wire: ToolFunctionWire) -> Result<Self, Self::Error> {
        Ok(Self {
            name: scannable_text(wire.name)?,
            arguments: scannable_text(wire.arguments)?,
        })
    }
}

// A client can post any JSON shape here, and a value that failed to parse would be a call nobody scans, so non-strings scan as their JSON text.
fn scannable_text(value: Option<Box<RawValue>>) -> Result<Option<String>, serde_json::Error> {
    let Some(raw) = value else { return Ok(None) };
    if raw.get().starts_with('"') {
        return serde_json::from_str(raw.get()).map(Some);
    }
    Ok(Some(raw.get().to_owned()))
}

#[derive(Debug, Clone, Copy)]
pub struct ToolScan<'scan> {
    pub direction: Direction,
    pub metadata: &'scan ScanMetadata,
    pub call_id: Option<&'scan str>,
    pub start_time: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScannedText {
    NameAndArguments,
    ArgumentsOnly,
    NameOnly,
}

// Mirrors LiteLLM's fn _scan_tool_calls_for_guardrail; masked arguments are written into the tool calls.
pub async fn scan_tool_calls_for_guardrail(
    guardrail: &PanwPrismaAirs,
    tool_calls: &mut [ToolCall],
    scan: &ToolScan<'_>,
    audit: &mut GuardrailAudit,
) -> Result<(), GuardrailHttpError> {
    for tool_call in tool_calls {
        let Some(function) = tool_call.function.as_mut() else {
            continue;
        };
        let Some((text, scanned)) = tool_scan_text(function) else {
            continue;
        };
        let verdict = call_panw_api(
            guardrail,
            ScanRequest {
                content: &text,
                direction: scan.direction,
                metadata: Some(scan.metadata),
                call_id: scan.call_id,
                tool_event: None,
            },
        )
        .await;
        if verdict.failure.is_some() {
            handle_api_error_with_logging(
                guardrail,
                &verdict,
                audit,
                scan.start_time,
                scan.direction,
            )?;
            continue;
        }
        record_scan_id(guardrail, audit, &verdict, scan.direction);
        let masked = masked_tool_call_arguments(&verdict, scan.direction, scanned);
        apply_tool_verdict(guardrail, function, &verdict, masked, scan.direction)?;
    }
    Ok(())
}

// Tool name and arguments go out as plain text: AIRS' tool_event schema only accepts ecosystem "mcp".
fn tool_scan_text(function: &ToolFunction) -> Option<(String, ScannedText)> {
    let function_name = function.name.as_deref().filter(|text| !text.is_empty());
    let function_args = function
        .arguments
        .as_deref()
        .filter(|text| !text.trim().is_empty());
    let (text, scanned) = match (function_name, function_args) {
        (Some(called), Some(given)) => {
            (format!("{called}\n{given}"), ScannedText::NameAndArguments)
        }
        (None, Some(given)) => (given.to_owned(), ScannedText::ArgumentsOnly),
        (Some(called), None) => (called.to_owned(), ScannedText::NameOnly),
        (None, None) => return None,
    };
    (!text.trim().is_empty()).then_some((text, scanned))
}

// Allowed content takes the policy's masks whatever the mask flags say; blocked content only when the flag for its side is on.
fn apply_tool_verdict(
    guardrail: &PanwPrismaAirs,
    function: &mut ToolFunction,
    verdict: &ScanVerdict,
    masked: Option<String>,
    direction: Direction,
) -> Result<(), GuardrailHttpError> {
    let mask_content = &guardrail.settings.masking.content;
    let blocked_may_mask = match direction {
        Direction::Response => mask_content.response_content,
        Direction::Request => mask_content.request_content,
    };
    match masked {
        Some(arguments) if verdict.action == Action::Allow || blocked_may_mask => {
            function.arguments = Some(arguments);
            Ok(())
        }
        _ if verdict.action == Action::Allow => Ok(()),
        _ => Err(GuardrailHttpError {
            status: 400,
            detail: Box::new(build_error_detail(guardrail, verdict, direction)),
        }),
    }
}

// Recovers the arguments slice of a masked scan, or None when it cannot be applied.
fn masked_tool_call_arguments(
    verdict: &ScanVerdict,
    direction: Direction,
    scanned: ScannedText,
) -> Option<String> {
    let masked_text = get_masked_text(&verdict.masks, direction == Direction::Response)?;
    let arguments = match scanned {
        ScannedText::NameOnly => return None,
        ScannedText::ArgumentsOnly => masked_text,
        ScannedText::NameAndArguments => masked_text.split_once('\n')?.1,
    };
    (!arguments.is_empty()).then(|| arguments.to_owned())
}

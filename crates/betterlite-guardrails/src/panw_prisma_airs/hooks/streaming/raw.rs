use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{StreamChunk, StreamError};
use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit};
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_detail::{GuardrailHttpError, build_error_detail};
use crate::panw_prisma_airs::error_handling::handle_api_error_with_logging;
use crate::panw_prisma_airs::hooks::audit_bridge;
use crate::panw_prisma_airs::hooks::data::{self, Data};
use crate::panw_prisma_airs::hooks::scan_metadata;
use crate::panw_prisma_airs::payload::ScanRequest;
use crate::panw_prisma_airs::scan::call_panw_api;
use crate::panw_prisma_airs::verdict::{Action, ScanVerdict};
use crate::prisma_airs_masking::get_masked_text;

// Mirrors LiteLLM's fn _extract_text_from_sse_bytes: lines split on \n only, invalid UTF-8 becomes U+FFFD, strip is Python's.
#[must_use]
pub fn extract_text_from_sse_bytes(chunks: &[StreamChunk]) -> String {
    let mut joined = Vec::new();
    for chunk in chunks {
        if let StreamChunk::Bytes(bytes) = chunk {
            joined.extend_from_slice(bytes);
        }
    }
    let is_blank =
        |character: char| character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character);
    let mut parts = Vec::new();
    for raw_line in joined.split(|byte| *byte == b'\n') {
        let mut line = String::with_capacity(raw_line.len());
        for piece in raw_line.utf8_chunks() {
            line.push_str(piece.valid());
            if !piece.invalid().is_empty() {
                line.push('\u{FFFD}');
            }
        }
        let Some(payload) = line.trim_matches(is_blank).strip_prefix("data: ") else {
            continue;
        };
        let Ok(event) = serde_json::from_str::<Data>(payload) else {
            continue;
        };
        if data::text(&event, "type") != Some("content_block_delta") {
            continue;
        }
        let Some(GuardrailValue::Object(delta)) = event.get("delta") else {
            continue;
        };
        if data::text(delta, "type") == Some("text_delta")
            && let Some(GuardrailValue::Text(text)) = delta.get("text")
        {
            parts.push(text.clone());
        }
    }
    parts.concat()
}

// Mirrors LiteLLM's fn _extract_text_from_streaming_events for /v1/responses events.
#[must_use]
pub fn extract_event_text(chunks: &[StreamChunk]) -> String {
    let mut parts = Vec::new();
    for chunk in chunks {
        let StreamChunk::Event(GuardrailValue::Object(event)) = chunk else {
            continue;
        };
        if data::text(event, "type") == Some("response.output_text.delta") {
            if let Some(GuardrailValue::Text(delta)) = event.get("delta") {
                parts.push(delta.clone());
            }
            continue;
        }
        if data::text(event, "object") == Some("chat.completion.chunk") {
            push_chat_delta_text(event, &mut parts);
        }
    }
    if parts.is_empty() {
        push_done_text(chunks, &mut parts);
    }
    parts.concat()
}

fn push_chat_delta_text(event: &BTreeMap<String, GuardrailValue>, parts: &mut Vec<String>) {
    let Some(GuardrailValue::Array(choices)) = event.get("choices") else {
        return;
    };
    for choice in choices {
        let GuardrailValue::Object(choice) = choice else {
            continue;
        };
        let Some(GuardrailValue::Object(delta)) = choice.get("delta") else {
            continue;
        };
        if let Some(GuardrailValue::Text(content)) = delta.get("content") {
            parts.push(content.clone());
        }
    }
}

// The fallback: response.output_text.done carries the full text when no deltas were captured.
fn push_done_text(chunks: &[StreamChunk], parts: &mut Vec<String>) {
    for chunk in chunks {
        let StreamChunk::Event(GuardrailValue::Object(event)) = chunk else {
            continue;
        };
        if data::text(event, "type") == Some("response.output_text.done")
            && let Some(GuardrailValue::Text(text)) = event.get("text")
        {
            parts.push(text.clone());
        }
    }
}

// Mirrors LiteLLM's fn _scan_raw_streaming_text; masking cannot reach raw bytes, so a mask that was returned blocks.
pub async fn scan_raw_streaming_text(
    guardrail: &PanwPrismaAirs,
    data: &Data,
    text: &str,
    audit: &mut GuardrailAudit,
    start_time: SystemTime,
) -> Result<(), StreamError> {
    if text.trim().is_empty() {
        return Ok(());
    }
    let Some(verdict) = scan_raw_verdict(guardrail, data, text, audit, start_time).await? else {
        return Ok(());
    };
    if verdict.action == Action::Block {
        return block_raw(guardrail, &verdict);
    }
    audit_bridge::record_response_success(guardrail, audit, &verdict, start_time);
    Ok(())
}

// Ok(None) means a transient error was waved through by _handle_api_error_with_logging.
async fn scan_raw_verdict(
    guardrail: &PanwPrismaAirs,
    data: &Data,
    text: &str,
    audit: &mut GuardrailAudit,
    start_time: SystemTime,
) -> Result<Option<ScanVerdict>, StreamError> {
    let metadata =
        scan_metadata::scan_metadata(data).map_err(|source| StreamError::Data { source })?;
    let verdict = call_panw_api(
        guardrail,
        ScanRequest {
            content: text,
            direction: Direction::Response,
            metadata: Some(&metadata),
            call_id: data::call_id(data),
            tool_event: None,
        },
    )
    .await;
    if verdict.failure.is_some() {
        handle_api_error_with_logging(guardrail, &verdict, audit, start_time, Direction::Response)?;
        return Ok(None);
    }
    Ok(Some(verdict))
}

fn block_raw(guardrail: &PanwPrismaAirs, verdict: &ScanVerdict) -> Result<(), StreamError> {
    let masked = get_masked_text(&verdict.masks, true).filter(|text| !text.is_empty());
    if masked.is_some() && guardrail.settings.masking.content.response_content {
        tracing::warn!(
            "PANW Prisma AIRS: mask_response_content is configured but cannot be applied to raw streaming responses (/v1/messages or /v1/responses). Blocking response instead."
        );
    }
    let detail = build_error_detail(guardrail, verdict, Direction::Response);
    Err(StreamError::Http(GuardrailHttpError {
        status: 400,
        detail: Box::new(detail),
    }))
}

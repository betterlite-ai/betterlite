use std::time::SystemTime;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_message_scope::StreamItem;
use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit};
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_detail::GuardrailHttpError;
use crate::panw_prisma_airs::error_handling::handle_api_error_with_logging;
use crate::panw_prisma_airs::hooks::audit_bridge;
use crate::panw_prisma_airs::hooks::context::HookContext;
use crate::panw_prisma_airs::hooks::data::Data;
use crate::panw_prisma_airs::hooks::dedup::{self, ScanType};
use crate::panw_prisma_airs::hooks::error;
use crate::panw_prisma_airs::hooks::gate;

use super::{StreamChunk, StreamError, StreamingResponse, assembly, frames, into_items, raw};

// Mirrors LiteLLM's fn async_post_call_streaming_iterator_hook; errors are frames, not raised.
pub async fn streaming_iterator_hook(
    guardrail: &PanwPrismaAirs,
    data: &mut Data,
    stream: StreamingResponse<'_>,
    audit: &mut GuardrailAudit,
    context: &HookContext<'_>,
) -> Result<Vec<StreamItem>, GuardrailHttpError> {
    let should_run = gate::should_run_guardrail(data, GuardrailEventHooks::PostCall, context)
        .map_err(|source| error::internal(&source, Direction::Response, guardrail))?;
    if !should_run {
        return Ok(into_items(stream.chunks));
    }
    if dedup::check_and_mark_scanned(data, ScanType::Streaming, context.new_id)
        .map_err(|source| error::internal(&source, Direction::Response, guardrail))?
    {
        return Ok(into_items(stream.chunks));
    }
    tracing::info!("PANW Prisma AIRS: Running post-call streaming scan");
    let start_time = (guardrail.now)();
    let outcome = scan_stream(guardrail, data, stream, audit, start_time).await;
    audit_bridge::flush(data, audit, guardrail.settings.guardrail_name.as_str());
    match outcome {
        Ok(items) => Ok(items),
        Err(error) => Ok(vec![frames::error_item(
            guardrail.settings.guardrail_name.as_str(),
            error,
        )]),
    }
}

// LiteLLM routes on the first chunk: raw bytes, /v1/responses events, or ModelResponse chunks.
async fn scan_stream(
    guardrail: &PanwPrismaAirs,
    data: &Data,
    stream: StreamingResponse<'_>,
    audit: &mut GuardrailAudit,
    start_time: SystemTime,
) -> Result<Vec<StreamItem>, StreamError> {
    let raw_text = match stream.chunks.first() {
        None => return Ok(Vec::new()),
        Some(StreamChunk::Bytes(_)) => Some(raw::extract_text_from_sse_bytes(&stream.chunks)),
        Some(StreamChunk::Event(_)) => Some(raw::extract_event_text(&stream.chunks)),
        Some(StreamChunk::Model(_)) => None,
    };
    let Some(text) = raw_text else {
        return scan_assembled(guardrail, data, stream, audit, start_time).await;
    };
    raw::scan_raw_streaming_text(guardrail, data, &text, audit, start_time).await?;
    Ok(into_items(stream.chunks))
}

// LiteLLM's stream_chunk_builder path: a None assembly yields the chunks untouched.
async fn scan_assembled(
    guardrail: &PanwPrismaAirs,
    data: &Data,
    stream: StreamingResponse<'_>,
    audit: &mut GuardrailAudit,
    start_time: SystemTime,
) -> Result<Vec<StreamItem>, StreamError> {
    let mut chunks = Vec::with_capacity(stream.chunks.len());
    for chunk in &stream.chunks {
        if let StreamChunk::Model(value) = chunk {
            chunks.push(value.clone());
        }
    }
    let assembled = stream
        .assembler
        .assemble(&chunks)
        .map_err(|source| StreamError::Assembly { source })?;
    let Some(mut response) = assembled else {
        return Ok(into_items(stream.chunks));
    };
    let scan = assembly::scan_streaming_response(guardrail, data, &mut response).await?;
    if scan.verdict.failure.is_some() {
        handle_api_error_with_logging(
            guardrail,
            &scan.verdict,
            audit,
            start_time,
            Direction::Response,
        )?;
        return Ok(into_items(stream.chunks));
    }
    audit_bridge::record_response_success(guardrail, audit, &scan.verdict, start_time);
    if !scan.modified {
        return Ok(into_items(stream.chunks));
    }
    let rebuilt = stream
        .assembler
        .reassemble(&response)
        .map_err(|source| StreamError::Assembly { source })?;
    Ok(into_items(rebuilt))
}

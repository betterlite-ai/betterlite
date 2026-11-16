use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use tokio::sync::mpsc;

use crate::anthropic_sse::is_raw_sse_stream::is_raw_sse_stream;
use crate::anthropic_sse::sse_chunks_from_response::sse_chunks_from_response;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_message_scope::{Object, StreamItem};
use crate::tool_permission::ToolPermissionGuardrail;

use super::buffer_overflow::{StreamBufferLimits, buffer_overflow_failure};
use super::error::ResponseToolError;
use super::evaluate::{DeniedToolCall, evaluate_tool_calls};
use super::extraction::extract_response_tools;
use super::openai_rewrite::rewrite_openai_responses;
use super::reconstruct::reconstruct_chunk;
use super::stream_assembly::{assemble_anthropic_stream, assemble_openai_chunks, rewritten_record};
use super::stream_services::StreamServices;
use super::unparseable::unparseable_stream_failure;

// LiteLLM's async_post_call_streaming_iterator_hook: every chunk is buffered before the response is checked
pub async fn post_call_streaming_iterator_hook(
    guardrail: &ToolPermissionGuardrail,
    limits: StreamBufferLimits,
    services: &StreamServices<'_>,
    mut input: mpsc::Receiver<StreamItem>,
    output: mpsc::Sender<StreamItem>,
) -> Result<(), ResponseToolError> {
    let chunks = buffer_stream(guardrail, limits, &mut input).await?;
    // LiteLLM only builds a chunk list when no chunk is raw wire data; one raw chunk makes the whole stream raw.
    let raw = is_raw_sse_stream(&chunks);
    let assembled = if raw {
        None
    } else {
        assemble_openai_chunks(&chunks, services).await?
    };
    if let Some(mut response) = assembled {
        let denied = check_assembled_stream(guardrail, &response)?;
        rewrite_openai_responses(&mut response, &denied);
        let reconstructed = reconstruct_chunk(&response);
        return relay(
            &output,
            vec![StreamItem::Json(GuardrailValue::Object(reconstructed))],
        )
        .await;
    }
    // A raw stream is assembled as Anthropic SSE; any assembly failure reads as no response to check.
    let Some(mut assembled) = assemble_anthropic_stream(&chunks, services).await else {
        if raw {
            return Err(ResponseToolError::Guardrail(unparseable_stream_failure(
                &guardrail.args.name,
            )));
        }
        return relay(&output, chunks).await;
    };
    let denied = check_assembled_stream(guardrail, &assembled.response)?;
    if denied.is_empty() {
        return relay(&output, chunks).await;
    }
    // The assembled Anthropic stream is an OpenAI-shaped response, so LiteLLM rewrites it with the OpenAI rewrite
    // before translating it back to Anthropic SSE.
    rewrite_openai_responses(&mut assembled.response, &denied);
    let record = rewritten_record(&assembled.response, &assembled.record)?;
    let frames = sse_chunks_from_response(&record)?;
    relay(&output, frames.into_iter().map(StreamItem::Bytes).collect()).await
}

// LiteLLM collects every chunk; the configured ceiling refuses the chunk that would exceed it.
async fn buffer_stream(
    guardrail: &ToolPermissionGuardrail,
    limits: StreamBufferLimits,
    input: &mut mpsc::Receiver<StreamItem>,
) -> Result<Vec<StreamItem>, ResponseToolError> {
    let mut chunks = Vec::new();
    while let Some(item) = input.recv().await {
        if chunks.len() == limits.max_chunks {
            return Err(ResponseToolError::Guardrail(buffer_overflow_failure(
                &guardrail.args.name,
                limits.max_chunks,
            )));
        }
        chunks.push(item);
    }
    Ok(chunks)
}

// LiteLLM's _check_assembled_stream: the response-side check runs for every assembled stream, whatever the request said
pub fn check_assembled_stream(
    guardrail: &ToolPermissionGuardrail,
    response: &Object,
) -> Result<Vec<DeniedToolCall>, GuardrailFailure> {
    tracing::debug!("Tool Permission Guardrail: Checking response");
    let calls = extract_response_tools(response);
    if calls.is_empty() {
        tracing::debug!("Tool Permission Guardrail: No tool uses found");
        return Ok(Vec::new());
    }
    tracing::debug!(
        count = calls.len(),
        "Tool Permission Guardrail: Found tool calls"
    );
    let denied = evaluate_tool_calls(guardrail, &calls)?;
    if denied.is_empty() {
        tracing::debug!("Tool Permission Guardrail Post-Call Hook: All tools allowed");
    }
    Ok(denied)
}

// A closed output means the client is gone, so the rest of the stream is dropped without failing the hook.
async fn relay(
    output: &mpsc::Sender<StreamItem>,
    items: Vec<StreamItem>,
) -> Result<(), ResponseToolError> {
    for item in items {
        if output.send(item).await.is_err() {
            break;
        }
    }
    Ok(())
}

mod tests;

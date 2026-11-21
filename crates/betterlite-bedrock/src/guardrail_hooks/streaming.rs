use std::time::Instant;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::routes::CallType;
use betterlite_prompts::response_tool_calls::structured::RouteCallTypes;

use super::chunks::{StreamChunk, is_raw_sse_stream};
use super::errors::{BlockedChat, BlockedMessage, HookFailure, ModifyFailure, detail_message};
use super::frames::error_frames;
use super::keepalive::keepalive_ping_has_fired;
use super::response::{chat_chunk, model_response_text, response_model, response_usage};
use super::runtime::{HookRuntime, RequestData, ScanCall, ScanSource, UnifiedCall};
use super::{StreamingSettings, is_responses_api_route, streams_incrementally};

pub struct StreamHook<'hook, R: HookRuntime, T: RouteCallTypes<CallType>> {
    pub runtime: &'hook R,
    pub settings: &'hook StreamingSettings,
    pub routes: &'hook T,
    pub request_route: Option<&'hook str>,
    pub data: &'hook mut RequestData,
    pub chunks: Vec<StreamChunk>,
}

struct StreamContext<'context> {
    settings: &'context StreamingSettings,
    started: Instant,
    raw_sse: bool,
}

// LiteLLM's async_post_call_streaming_iterator_hook: incremental and Responses routes delegate, the rest buffer the whole stream.
pub async fn streaming_iterator_hook<R, T>(
    hook: StreamHook<'_, R, T>,
) -> Result<Vec<StreamChunk>, HookFailure>
where
    R: HookRuntime,
    T: RouteCallTypes<CallType> + Sync,
{
    let StreamHook {
        runtime,
        settings,
        routes,
        request_route,
        data,
        chunks,
    } = hook;
    if streams_incrementally(settings) {
        return runtime
            .delegate(UnifiedCall {
                request_data: data,
                chunks,
                buffer_until_moderated_default: false,
            })
            .await;
    }
    if is_responses_api_route(request_route, routes) {
        return runtime
            .delegate(UnifiedCall {
                request_data: data,
                chunks,
                buffer_until_moderated_default: true,
            })
            .await;
    }
    let started = Instant::now();
    scan_buffered_stream(runtime, settings, data, chunks, started).await
}

async fn scan_buffered_stream<R>(
    runtime: &R,
    settings: &StreamingSettings,
    data: &mut RequestData,
    chunks: Vec<StreamChunk>,
    started: Instant,
) -> Result<Vec<StreamChunk>, HookFailure>
where
    R: HookRuntime,
{
    let raw_sse = is_raw_sse_stream(&chunks);
    let context = StreamContext {
        settings,
        started,
        raw_sse,
    };
    let Some(mut assembled) = runtime.assemble(&chunks, raw_sse).await else {
        if raw_sse {
            let message = format!(
                "{}: streamed response could not be assembled for scanning, blocking it",
                settings.guardrail_name
            );
            return Ok(error_frames(&message));
        }
        return Ok(chunks);
    };
    let before = model_response_text(&assembled);
    let messages = runtime.request_messages(data);
    let scan = ScanCall {
        source: ScanSource::Output,
        messages: messages.as_deref(),
        response: Some(&assembled),
        request_data: &mut *data,
        event: GuardrailEventHooks::PostCall,
    };
    let scan_response = match runtime.scan(scan).await {
        Ok(scan_response) => scan_response,
        Err(failure) => return stream_failure(runtime, &context, &assembled, failure),
    };
    runtime.mask_response(&mut assembled, &scan_response);
    if raw_sse {
        return raw_output(runtime, &assembled, chunks, &before);
    }
    match chat_chunk(&assembled) {
        Some(chunk) => Ok(vec![chunk]),
        None => Err(HookFailure::Failed {
            message: "the assembled response could not be replayed as a chunk".to_owned(),
        }),
    }
}

// A raw stream whose text did not change keeps the frames it already had, since re-encoding would rewrite them.
fn raw_output<R>(
    runtime: &R,
    assembled: &GuardrailValue,
    chunks: Vec<StreamChunk>,
    before: &str,
) -> Result<Vec<StreamChunk>, HookFailure>
where
    R: HookRuntime,
{
    if model_response_text(assembled) == before {
        return Ok(chunks);
    }
    Ok(runtime
        .reassemble_sse(assembled)?
        .into_iter()
        .map(StreamChunk::Bytes)
        .collect())
}

fn stream_failure<R>(
    runtime: &R,
    context: &StreamContext<'_>,
    assembled: &GuardrailValue,
    failure: HookFailure,
) -> Result<Vec<StreamChunk>, HookFailure>
where
    R: HookRuntime,
{
    match failure {
        HookFailure::ModifyResponse(failure) => block_output(runtime, context, assembled, *failure),
        HookFailure::Rejected { status, detail } => {
            rejected_output(runtime, context, detail, status)
        }
        other @ HookFailure::Failed { .. } => Err(other),
    }
}

// A policy block is the only 400 with a structured detail; a service failure keeps its status only while the headers are unflushed.
fn rejected_output<R>(
    runtime: &R,
    context: &StreamContext<'_>,
    detail: GuardrailValue,
    status: u16,
) -> Result<Vec<StreamChunk>, HookFailure>
where
    R: HookRuntime,
{
    let is_block = context.raw_sse && status == 400 && matches!(detail, GuardrailValue::Object(_));
    let headers_flushed = keepalive_ping_has_fired(
        runtime.elapsed(context.started),
        context.settings.keepalive_interval_seconds,
    );
    if !context.raw_sse || (!is_block && !headers_flushed) {
        return Err(HookFailure::Rejected { status, detail });
    }
    let message = detail_message(&detail);
    let text = if is_block {
        message
    } else {
        format!("{status}: {message}")
    };
    Ok(error_frames(&text))
}

// Streaming cannot raise a block: the frames are already out, so the replacement carries the model and usage the caller saw.
fn block_output<R>(
    runtime: &R,
    context: &StreamContext<'_>,
    assembled: &GuardrailValue,
    failure: ModifyFailure,
) -> Result<Vec<StreamChunk>, HookFailure>
where
    R: HookRuntime,
{
    if !context.raw_sse {
        let block = BlockedChat {
            message: failure.message,
            model: failure.model,
            usage: response_usage(assembled),
        };
        return runtime.block_chat_chunks(&block);
    }
    let model = response_model(assembled)
        .filter(|model| !model.is_empty())
        .map(ToOwned::to_owned)
        .or(failure.model)
        .unwrap_or_default();
    let block = BlockedMessage {
        message: failure.message,
        model,
        original_response: failure
            .original_response
            .unwrap_or_else(|| assembled.clone()),
    };
    let frames = runtime.block_sse_chunks(&block)?;
    Ok(frames.into_iter().map(StreamChunk::Bytes).collect())
}

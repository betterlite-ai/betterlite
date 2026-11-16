use betterlite_core::rules::{RulePhase, evaluate};
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::provider_chunk_decode::creator::chunk_creator;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::stream_chunk_shaping::creation::model_response_creator;
use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamChunk, Usage};
use betterlite_streaming::stream_chunk_shaping::usage_totals::calculate_total_usage;

use super::ChatStream;
use super::config::CallMode;
use super::error::ChatStreamCause;
use super::ports::SharedRules;
use super::{empty, hooks, json, mcp};

// What one source chunk became: a chunk to return, silence, or the end of the stream.
pub(super) enum Step {
    Chunk(StreamChunk),
    Skip,
    End,
}

// __next__ ignores b"" while __anext__ ignores the literal "None".
pub(super) fn skip_source_chunk(mode: CallMode, chunk: &ProviderChunk) -> bool {
    if matches!(mode, CallMode::Async) {
        return matches!(chunk, ProviderChunk::Text(text) if text.as_str() == "None");
    }
    matches!(chunk, ProviderChunk::Bytes(bytes) if bytes.is_empty())
}

pub(super) async fn process<S>(
    stream: &mut ChatStream<'_, S>,
    raw: ProviderChunk,
) -> Result<Step, ChatStreamCause> {
    let created = match chunk_creator(&mut stream.shaper, &mut stream.decode, &raw) {
        Ok(Some(chunk)) => chunk,
        Ok(None) => return Ok(Step::Skip),
        Err(ProviderChunkError::StopIteration) => return Ok(Step::End),
        Err(error) => return Err(ChatStreamCause::Chunk(error)),
    };
    // chunk_creator names the shaped chunk after the call's model; every later creator call uses the logged one.
    stream.shaper.model.clone_from(&stream.logged_model);
    mark_started(stream);
    match stream.config.mode {
        CallMode::Blocking => blocking(stream, created),
        CallMode::Async => asynchronous(stream, created).await,
    }
}

fn mark_started<S>(stream: &mut ChatStream<'_, S>) {
    if stream.logging_started {
        return;
    }
    stream.logging_started = true;
    stream.ports.logging.started(stream.ports.clock.now());
}

fn blocking<S>(
    stream: &mut ChatStream<'_, S>,
    mut chunk: StreamChunk,
) -> Result<Step, ChatStreamCause> {
    if !stream.config.logging.disable {
        stream.ports.logging.log_chunk(&chunk, stream.cache_hit);
    }
    accumulate_content(stream, &chunk);
    evaluate_post_call(stream)?;
    stream.shaper.chunks.push(chunk.clone());
    first_chunk_mcp(stream, &mut chunk);
    if chunk.usage.is_some() {
        let Some(rebuilt) = usage_rebuilt(stream, chunk)? else {
            return Ok(Step::Skip);
        };
        chunk = rebuilt;
    }
    blocking_final(stream, &mut chunk)?;
    Ok(Step::Chunk(chunk))
}

async fn asynchronous<S>(
    stream: &mut ChatStream<'_, S>,
    mut chunk: StreamChunk,
) -> Result<Step, ChatStreamCause> {
    accumulate_content(stream, &chunk);
    evaluate_post_call(stream)?;
    first_chunk_mcp(stream, &mut chunk);
    let has_usage = chunk.usage.is_some();
    stream.shaper.chunks.push(chunk.clone());
    if has_usage {
        let Some(rebuilt) = usage_rebuilt(stream, chunk)? else {
            return Ok(Step::Skip);
        };
        chunk = rebuilt;
    }
    let ready = async_final(stream, chunk).await?;
    Ok(Step::Chunk(ready))
}

fn accumulate_content<S>(stream: &mut ChatStream<'_, S>, chunk: &StreamChunk) {
    let content = chunk
        .choices
        .first()
        .and_then(|choice| choice.delta.content.as_deref())
        .unwrap_or_default();
    stream.shaper.response_uptil_now.push_str(content);
}

fn evaluate_post_call<S>(stream: &mut ChatStream<'_, S>) -> Result<(), ChatStreamCause> {
    let rules = SharedRules(stream.ports.rules);
    evaluate(
        RulePhase::PostCall,
        &rules,
        Some(&stream.shaper.response_uptil_now),
        &stream.config.model,
    )?;
    Ok(())
}

// _add_mcp_list_tools_to_first_chunk runs on the first chunk that carries choices.
fn first_chunk_mcp<S>(stream: &mut ChatStream<'_, S>, chunk: &mut StreamChunk) {
    if stream.shaper.progress.sent_first_chunk || chunk.choices.is_empty() {
        return;
    }
    mcp::add_list_tools(chunk, mcp::metadata(stream.config.mcp_metadata.as_ref()));
    stream.shaper.progress.sent_first_chunk = true;
}

// The usage branch: move usage to hidden params, strip it, and drop the chunk when nothing is left.
fn usage_rebuilt<S>(
    stream: &mut ChatStream<'_, S>,
    mut chunk: StreamChunk,
) -> Result<Option<StreamChunk>, ChatStreamCause> {
    if let Some(usage) = &chunk.usage {
        let encoded: JsonValue = json::bridge(usage)?;
        chunk.hidden_params.insert("usage".to_owned(), encoded);
    }
    let object = json::chunk_object(&chunk)?;
    let rebuilt = model_response_creator(
        &mut stream.shaper,
        Some(&object),
        Some(&chunk.hidden_params),
    )?;
    if empty::is_empty(&rebuilt) {
        return Ok(None);
    }
    Ok(Some(rebuilt))
}

// __next__'s final chunk gets the running usage and the MCP metadata when stream_options is unset.
fn blocking_final<S>(
    stream: &mut ChatStream<'_, S>,
    chunk: &mut StreamChunk,
) -> Result<(), ChatStreamCause> {
    if !stream.shaper.progress.sent_last_chunk || stream.config.stream_options.is_some() {
        return Ok(());
    }
    insert_total_usage(chunk, &stream.shaper.chunks)?;
    mcp::add_final_metadata(chunk, mcp::metadata(stream.config.mcp_metadata.as_ref()));
    Ok(())
}

// __anext__'s final chunk: hidden usage, then the deployment hook, then the MCP metadata.
async fn async_final<S>(
    stream: &mut ChatStream<'_, S>,
    mut chunk: StreamChunk,
) -> Result<StreamChunk, ChatStreamCause> {
    if !stream.shaper.progress.sent_last_chunk {
        return Ok(chunk);
    }
    if stream.config.stream_options.is_none() {
        insert_total_usage(&mut chunk, &stream.shaper.chunks)?;
    }
    let mut hooked = hooks::run(&stream.ports, chunk).await;
    mcp::add_final_metadata(
        &mut hooked,
        mcp::metadata(stream.config.mcp_metadata.as_ref()),
    );
    Ok(hooked)
}

pub(super) fn insert_total_usage(
    chunk: &mut StreamChunk,
    chunks: &[StreamChunk],
) -> Result<(), ChatStreamCause> {
    let usage: Usage = calculate_total_usage(chunks);
    let encoded: JsonValue = json::bridge(&usage)?;
    chunk.hidden_params.insert("usage".to_owned(), encoded);
    Ok(())
}

use betterlite_core::rules::{RulePhase, evaluate};
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::provider_chunk_decode::creator::chunk_creator;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::stream_chunk_shaping::StreamShapeError;
use betterlite_streaming::stream_chunk_shaping::creation::model_response_creator;
use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamChunk};
use betterlite_streaming::stream_chunk_shaping::usage_totals::calculate_total_usage;
use serde_json::{from_value, to_value};

use super::ChatStream;
use super::config::CallMode;
use super::error::ChatStreamCause;
use super::ports::{ChunkSource, SharedRules};
use super::{empty, hooks, json, mcp};

// What one source chunk became: a chunk to return, silence, or the end of the stream.
pub(super) enum Step {
    Chunk(Box<StreamChunk>),
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
) -> Result<Step, ChatStreamCause>
where
    S: ChunkSource,
{
    let outcome = chunk_creator(&mut stream.shaper, &mut stream.decode, &raw);
    // chunk_creator names the shaped chunk after the call's model; every later creator call uses the logged one.
    stream.shaper.model.clone_from(&stream.logged_model);
    let created = match outcome {
        Ok(Some(chunk)) => chunk,
        Ok(None) => return Ok(Step::Skip),
        Err(ProviderChunkError::StopIteration) => return Ok(Step::End),
        Err(error) => return Err(ChatStreamCause::Chunk(error)),
    };
    mark_started(stream);
    produced(stream, created).await
}

fn mark_started<S>(stream: &mut ChatStream<'_, S>) {
    if stream.logging_started {
        return;
    }
    stream.logging_started = true;
    stream.ports.logging.started(stream.ports.clock.now());
}

// __next__ and __anext__ run the same steps; only the per-chunk logging, the chunk order and the final step differ.
async fn produced<S>(
    stream: &mut ChatStream<'_, S>,
    mut chunk: StreamChunk,
) -> Result<Step, ChatStreamCause>
where
    S: ChunkSource,
{
    let mode = stream.config.mode;
    if matches!(mode, CallMode::Blocking) && !stream.config.logging.disable {
        stream.ports.logging.log_chunk(&chunk, stream.cache_hit);
    }
    accumulate_content(stream, &chunk);
    evaluate_post_call(stream)?;
    if matches!(mode, CallMode::Blocking) {
        stream.shaper.chunks.push(chunk.clone());
        first_chunk_mcp(stream, &mut chunk);
    } else {
        first_chunk_mcp(stream, &mut chunk);
        stream.shaper.chunks.push(chunk.clone());
    }
    if chunk.usage.is_some() {
        let Some(rebuilt) = usage_rebuilt(stream, &chunk)? else {
            return Ok(Step::Skip);
        };
        chunk = rebuilt;
    }
    match mode {
        CallMode::Blocking => blocking_final(stream, &mut chunk)?,
        CallMode::Async => chunk = async_final(stream, chunk).await?,
    }
    Ok(Step::Chunk(Box::new(chunk)))
}

fn accumulate_content<S>(stream: &mut ChatStream<'_, S>, chunk: &StreamChunk) {
    let content = chunk
        .choices
        .first()
        .and_then(|choice| choice.delta.content.as_deref())
        .unwrap_or_default();
    stream.shaper.response_uptil_now.push_str(content);
}

fn evaluate_post_call<S>(stream: &ChatStream<'_, S>) -> Result<(), ChatStreamCause> {
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
    mcp::add_list_tools(chunk, stream.config.mcp_metadata.as_ref());
    stream.shaper.progress.sent_first_chunk = true;
}

// The usage branch: rebuild the chunk without its usage, and drop it when nothing else is left.
fn usage_rebuilt<S>(
    stream: &mut ChatStream<'_, S>,
    chunk: &StreamChunk,
) -> Result<Option<StreamChunk>, ChatStreamCause> {
    let mut hidden = chunk.hidden_params.clone();
    // __anext__ passes the chunk's own hidden params through, so only __next__ adds the usage here.
    if matches!(stream.config.mode, CallMode::Blocking)
        && let Some(usage) = &chunk.usage
    {
        let serialized = to_value(usage).map_err(StreamShapeError::from)?;
        let encoded: JsonValue = from_value(serialized).map_err(StreamShapeError::from)?;
        hidden.insert("usage".to_owned(), encoded);
    }
    let object = json::chunk_object(chunk)?;
    let rebuilt = model_response_creator(&mut stream.shaper, Some(&object), Some(&hidden))?;
    if empty::is_empty(&rebuilt) {
        return Ok(None);
    }
    Ok(Some(rebuilt))
}

// __next__'s final chunk gets the running usage and the MCP metadata when stream_options is unset.
fn blocking_final<S>(
    stream: &ChatStream<'_, S>,
    chunk: &mut StreamChunk,
) -> Result<(), ChatStreamCause> {
    if !stream.shaper.progress.sent_last_chunk || stream.config.stream_options.is_some() {
        return Ok(());
    }
    insert_total_usage(chunk, &stream.shaper.chunks)?;
    mcp::add_final_metadata(chunk, stream.config.mcp_metadata.as_ref());
    Ok(())
}

// __anext__'s final chunk: hidden usage, then the deployment hook, then the MCP metadata.
async fn async_final<S>(
    stream: &ChatStream<'_, S>,
    mut chunk: StreamChunk,
) -> Result<StreamChunk, ChatStreamCause>
where
    S: ChunkSource,
{
    if !stream.shaper.progress.sent_last_chunk {
        return Ok(chunk);
    }
    if stream.config.stream_options.is_none() {
        insert_total_usage(&mut chunk, &stream.shaper.chunks)?;
    }
    let mut hooked = hooks::run(&stream.ports, chunk).await;
    mcp::add_final_metadata(&mut hooked, stream.config.mcp_metadata.as_ref());
    Ok(hooked)
}

pub(super) fn insert_total_usage(
    chunk: &mut StreamChunk,
    chunks: &[StreamChunk],
) -> Result<(), ChatStreamCause> {
    let usage = calculate_total_usage(chunks);
    let serialized = to_value(&usage).map_err(StreamShapeError::from)?;
    let encoded: JsonValue = from_value(serialized).map_err(StreamShapeError::from)?;
    chunk.hidden_params.insert("usage".to_owned(), encoded);
    Ok(())
}

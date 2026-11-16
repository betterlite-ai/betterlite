use std::collections::VecDeque;

use serde_json::value::RawValue;
use tokio::sync::mpsc::{Receiver, Sender};

use super::assembly::StreamAssembly;
use super::budget::Retention;
use super::coalesce::{Coalesced, Coalescer};
use super::error::{PresidioStreamError, StreamError};
use super::mask_anthropic::mask_anthropic_sse_stream;
use super::{StreamEvent, StreamRun};
use crate::cato::error::ModelStreamError;
use crate::presidio_response::Mode;
use crate::presidio_response::model::process_response_for_pii;

pub async fn send_chat_chunks<A>(
    out: &Sender<StreamEvent>,
    chunks: &[Box<RawValue>],
) -> Result<(), StreamError<A>>
where
    A: StreamAssembly,
{
    for chunk in chunks {
        out.send(StreamEvent::Chat(chunk.clone())).await?;
    }
    Ok(())
}

// Buffered chunks go out unmasked when a mixed stream shape makes reconstruction impossible
pub async fn flush_unmaskable_buffer<A>(
    all_chunks: &[Box<RawValue>],
    out: &Sender<StreamEvent>,
) -> Result<(), StreamError<A>>
where
    A: StreamAssembly,
{
    if all_chunks.is_empty() {
        return Ok(());
    }
    tracing::warn!(
        "Presidio apply_to_output: mixed stream detected (ModelResponseStream + unknown event). \
         Flushing {} buffered chunks without PII masking and switching to transparent passthrough.",
        all_chunks.len()
    );
    send_chat_chunks::<A>(out, all_chunks).await
}

// LiteLLM's _mask_buffered_model_response_stream
pub async fn mask_buffered_chunks<A>(
    run: &mut StreamRun<'_, A>,
    all_chunks: &[Box<RawValue>],
) -> Result<Vec<StreamEvent>, StreamError<A>>
where
    A: StreamAssembly,
{
    let messages = run.request_data.get("messages").map(|raw| &**raw);
    let assembled = run
        .assembly
        .stream_chunk_builder(all_chunks, messages)
        .await
        .map_err(|source| PresidioStreamError::Assembly { source })?;
    let Some(mut response) = assembled else {
        return Ok(all_chunks.iter().cloned().map(StreamEvent::Chat).collect());
    };
    process_response_for_pii(run.context, &mut response, run.request_data, Mode::Mask).await?;
    let streamed = run
        .assembly
        .convert_to_streaming(&response)
        .map_err(|source| PresidioStreamError::Assembly { source })?;
    Ok(vec![StreamEvent::Chat(streamed)])
}

// What the masking run holds while it waits for the end of the stream
#[derive(Default)]
struct MaskState {
    all_chunks: Vec<Box<RawValue>>,
    retention: Retention,
    passthrough: bool,
}

// Applies Presidio masking to a streamed answer (the apply_to_output path); only a failure that can leave the stream half-sent is swallowed
pub async fn stream_apply_output_masking<A>(
    run: &mut StreamRun<'_, A>,
    input: &mut Receiver<Result<StreamEvent, ModelStreamError>>,
) -> Result<(), StreamError<A>>
where
    A: StreamAssembly,
{
    let mut state = MaskState::default();
    let Err(error) = apply_output_masking(run, input, &mut state).await else {
        return Ok(());
    };
    if state.all_chunks.is_empty()
        || error.is_blocked()
        || matches!(
            error,
            PresidioStreamError::ConsumerClosed | PresidioStreamError::OverBudget { .. }
        )
    {
        return Err(error);
    }
    tracing::error!("Error masking streaming PII output: {error}");
    send_chat_chunks::<A>(run.out, &state.all_chunks).await
}

async fn apply_output_masking<A>(
    run: &mut StreamRun<'_, A>,
    input: &mut Receiver<Result<StreamEvent, ModelStreamError>>,
    state: &mut MaskState,
) -> Result<(), StreamError<A>>
where
    A: StreamAssembly,
{
    let mut stream = Coalescer {
        input,
        pending: Vec::new(),
        searching: true,
        ready: VecDeque::new(),
    };
    while let Some(item) = stream.coalesce_first_sse_frame().await {
        let coalesced = item.map_err(|source| PresidioStreamError::Upstream { source })?;
        let Coalesced::Event(event) = coalesced else {
            run.out.send(coalesced.into_event()).await?;
            continue;
        };
        if mask_event(run, state, event, &mut stream).await? {
            return Ok(());
        }
    }
    mask_remaining_chunks(run, &state.all_chunks, state.passthrough).await
}

// Handles one event after the first frame; true when the Anthropic path has taken the rest of the stream
async fn mask_event<A>(
    run: &mut StreamRun<'_, A>,
    state: &mut MaskState,
    event: StreamEvent,
    stream: &mut Coalescer<'_>,
) -> Result<bool, StreamError<A>>
where
    A: StreamAssembly,
{
    match event {
        StreamEvent::Chat(chunk) if !state.passthrough => {
            state.retention.accept::<A::Error>(chunk.get().len())?;
            state.all_chunks.push(chunk);
        }
        StreamEvent::Bytes(chunk)
            if !state.passthrough
                && state.all_chunks.is_empty()
                && run.assembly.is_anthropic_sse_stream(&chunk) =>
        {
            for masked in mask_anthropic_sse_stream(run, chunk, stream).await? {
                run.out.send(masked).await?;
            }
            return Ok(true);
        }
        StreamEvent::Bytes(chunk) => {
            state.passthrough = state.passthrough || state.all_chunks.is_empty();
            run.out.send(StreamEvent::Bytes(chunk)).await?;
        }
        StreamEvent::Chat(_) => run.out.send(event).await?,
        StreamEvent::Other(_) => {
            flush_unmaskable_buffer::<A>(&state.all_chunks, run.out).await?;
            state.all_chunks.clear();
            state.passthrough = true;
            run.out.send(event).await?;
        }
    }
    Ok(false)
}

async fn mask_remaining_chunks<A>(
    run: &mut StreamRun<'_, A>,
    all_chunks: &[Box<RawValue>],
    passthrough: bool,
) -> Result<(), StreamError<A>>
where
    A: StreamAssembly,
{
    if passthrough {
        tracing::warn!(
            "Presidio apply_to_output: streaming response was not a parsed chat completion stream \
             (raw non-Anthropic SSE passthrough or /v1/responses events). \
             Output PII masking was skipped for this response."
        );
        return Ok(());
    }
    if all_chunks.is_empty() {
        tracing::warn!(
            "Presidio apply_to_output: streaming response contained no \
             ModelResponseStream chunks (an empty upstream stream). \
             Output PII masking was skipped for this response."
        );
        return Ok(());
    }
    for masked in mask_buffered_chunks(run, all_chunks).await? {
        run.out.send(masked).await?;
    }
    Ok(())
}

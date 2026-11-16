use std::mem;

use serde_json::value::RawValue;
use tokio::sync::mpsc::Receiver;

use super::assembly::StreamAssembly;
use super::error::{PresidioStreamError, StreamError};
use super::mask::send_chat_chunks;
use super::unmask_sse::{Fields, PiiTokens, unmask_other_event, unmask_sse_bytes_chunk};
use super::{StreamEvent, StreamRun};
use crate::cato::error::ModelStreamError;
use crate::presidio_actions::request_data::pii_tokens;
use crate::presidio_response::Mode;
use crate::presidio_response::model::process_response_for_pii;
use crate::presidio_response::wire::ChatModelResponse;

// Copies the usage of the last chunk when the assembled response has none
pub fn preserve_usage_from_last_chunk(assembled: &mut ChatModelResponse, chunks: &[Box<RawValue>]) {
    if assembled
        .extra
        .get("usage")
        .is_some_and(|usage| usage.get().trim() != "null")
    {
        return;
    }
    let Some(last_chunk) = chunks.last() else {
        return;
    };
    let Ok(fields) = serde_json::from_str::<Fields>(last_chunk.get()) else {
        return;
    };
    if let Some(usage) = fields
        .get("usage")
        .filter(|usage| usage.get().trim() != "null")
    {
        assembled.extra.insert("usage".to_owned(), usage.clone());
    }
}

// Unmasks a streamed answer (the output_parse_pii path); a failure other than a closed reader sends the buffered chunks as they are
pub async fn stream_pii_unmasking<A>(
    run: &mut StreamRun<'_, A>,
    input: &mut Receiver<Result<StreamEvent, ModelStreamError>>,
) -> Result<(), StreamError<A>>
where
    A: StreamAssembly,
{
    let tokens = pii_tokens(run.request_data)?;
    let mut remaining = Vec::new();
    let Err(error) = unmask_stream(run, input, &tokens, &mut remaining).await else {
        return Ok(());
    };
    if matches!(error, PresidioStreamError::ConsumerClosed) {
        return Err(error);
    }
    tracing::error!("Error in PII streaming processing: {error}");
    send_chat_chunks::<A>(run.out, &remaining).await
}

async fn unmask_stream<A>(
    run: &mut StreamRun<'_, A>,
    input: &mut Receiver<Result<StreamEvent, ModelStreamError>>,
    tokens: &PiiTokens,
    remaining: &mut Vec<Box<RawValue>>,
) -> Result<(), StreamError<A>>
where
    A: StreamAssembly,
{
    let mut saw_non_chat_chunk = false;
    while let Some(received) = input.recv().await {
        match received.map_err(|source| PresidioStreamError::Upstream { source })? {
            StreamEvent::Chat(chunk) if saw_non_chat_chunk => {
                run.out.send(StreamEvent::Chat(chunk)).await?;
            }
            StreamEvent::Chat(chunk) => remaining.push(chunk),
            StreamEvent::Bytes(chunk) if tokens.is_empty() => {
                run.out.send(StreamEvent::Bytes(chunk)).await?;
            }
            StreamEvent::Bytes(chunk) => {
                let unmasked = unmask_sse_bytes_chunk(&chunk, tokens);
                run.out.send(StreamEvent::Bytes(unmasked)).await?;
            }
            StreamEvent::Other(chunk) => {
                // A mixed stream cannot be reassembled, so the buffered chat chunks go out in order first
                send_chat_chunks::<A>(run.out, &mem::take(remaining)).await?;
                let event = if tokens.is_empty() {
                    chunk
                } else {
                    unmask_other_event(chunk, tokens)
                        .map_err(|source| PresidioStreamError::Json { source })?
                };
                saw_non_chat_chunk = true;
                run.out.send(StreamEvent::Other(event)).await?;
            }
        }
    }
    if saw_non_chat_chunk || remaining.is_empty() {
        return Ok(());
    }
    assemble_and_unmask(run, remaining).await
}

async fn assemble_and_unmask<A>(
    run: &mut StreamRun<'_, A>,
    remaining: &[Box<RawValue>],
) -> Result<(), StreamError<A>>
where
    A: StreamAssembly,
{
    let messages = run.request_data.get("messages").map(|raw| &**raw);
    let assembled = run
        .assembly
        .stream_chunk_builder(remaining, messages)
        .await
        .map_err(|source| PresidioStreamError::Assembly { source })?;
    let Some(mut response) = assembled else {
        return send_chat_chunks::<A>(run.out, remaining).await;
    };
    preserve_usage_from_last_chunk(&mut response, remaining);
    process_response_for_pii(run.context, &mut response, run.request_data, Mode::Unmask).await?;
    let streamed = run
        .assembly
        .convert_to_streaming(&response)
        .map_err(|source| PresidioStreamError::Assembly { source })?;
    run.out.send(StreamEvent::Chat(streamed)).await?;
    Ok(())
}

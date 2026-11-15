use std::collections::VecDeque;
use std::mem;

use betterlite_bedrock::invoke_events::error::InvokeStreamError;
use betterlite_http::error::HttpError;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use betterlite_openai::aleph_alpha::lines::LineDecoder;
use betterlite_openai::compatible_line_stream::lines::Lines;
use betterlite_openai::compatible_line_stream::{
    CompatibleLineChunk, Mode, Outcome, ReceiveError, StreamError, decode_legacy_compatible_line,
    receive_failure,
};
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use futures_util::stream::{self, BoxStream, StreamExt as _};
use thiserror::Error;

pub type Bytes = BoxStream<'static, Result<Vec<u8>, HttpError>>;
pub type LineItems = BoxStream<'static, Result<String, ReceiveError<LineError>>>;
pub type LineChunks = BoxStream<'static, Result<CompatibleLineChunk, StreamError<LineError>>>;
pub type FakeChunks = BoxStream<'static, Result<GenericChunk, InvokeStreamError>>;
pub type DecodedChunks =
    BoxStream<'static, Result<GenericChunk, Box<dyn std::error::Error + Send + Sync>>>;

// httpx's chunk_size for aiter_bytes and iter_bytes on a custom decoder
pub const DECODER_CHUNK_BYTES: usize = 1024;

#[derive(Debug, Error)]
pub enum LineError {
    #[error("a streamed line is longer than {limit} bytes")]
    TooLong { limit: usize },
    #[error(transparent)]
    Decode(#[from] AlephAlphaError),
    #[error(transparent)]
    Transport(#[from] HttpError),
}

// B5840's signed SageMaker decoder plugs in here; LiteLLM's CustomStreamingDecoder reads bytes in 1024-byte pieces
pub trait StreamingDecoder: Send + Sync {
    fn decode(&self, bytes: Bytes) -> DecodedChunks;
}

pub enum CompatibleChatStream {
    Lines(LineChunks),
    Fake(FakeChunks),
    Decoder(DecodedChunks),
}

struct LineState {
    body: Bytes,
    decoder: LineDecoder,
    pending: VecDeque<String>,
    limit: usize,
    failure: Option<LineError>,
    done: bool,
}

async fn next_line(
    mut state: LineState,
) -> Option<(Result<String, ReceiveError<LineError>>, LineState)> {
    loop {
        if let Some(line) = state.pending.pop_front() {
            return Some((Ok(line), state));
        }
        if let Some(error) = state.failure.take() {
            state.done = true;
            return Some((Err(ReceiveError::Other(error)), state));
        }
        if state.done {
            return None;
        }
        match state.body.next().await {
            Some(Ok(bytes)) => {
                if let Err(error) = state.decoder.push(&bytes, &mut state.pending) {
                    state.failure = Some(match error {
                        AlephAlphaError::LineTooLong { limit } => LineError::TooLong { limit },
                        other => LineError::Decode(other),
                    });
                }
            }
            Some(Err(error)) => state.failure = Some(LineError::Transport(error)),
            None => {
                state.decoder.flush(&mut state.pending);
                state.done = true;
            }
        }
    }
}

// The blocking iterator reads with sync_stream=True, so its lines decode without the [DONE] end the async one has
fn sync_chunks(items: LineItems) -> LineChunks {
    items
        .filter_map(|item| {
            std::future::ready(match item {
                Ok(line) => match decode_legacy_compatible_line(&line, Mode::Sync) {
                    Outcome::Chunk(chunk) => Some(Ok(chunk)),
                    Outcome::End => None,
                },
                Err(error) => Some(Err(receive_failure(error, Mode::Sync))),
            })
        })
        .boxed()
}

#[must_use]
pub fn chunks(items: LineItems, mode: Mode) -> LineChunks {
    match mode {
        Mode::Async => Lines::from(items).boxed(),
        Mode::Sync => sync_chunks(items),
    }
}

#[must_use]
pub fn lines(body: Bytes, limit: usize, mode: Mode) -> LineChunks {
    let state = LineState {
        body,
        decoder: LineDecoder::new(limit),
        pending: VecDeque::new(),
        limit,
        failure: None,
        done: false,
    };
    let items: LineItems = stream::unfold(state, next_line).boxed();
    chunks(items, mode)
}

struct RechunkState {
    body: Bytes,
    pending: Vec<u8>,
    done: bool,
}

async fn next_chunk(mut state: RechunkState) -> Option<(Result<Vec<u8>, HttpError>, RechunkState)> {
    while !state.done && state.pending.len() < DECODER_CHUNK_BYTES {
        match state.body.next().await {
            Some(Ok(bytes)) => state.pending.extend_from_slice(&bytes),
            Some(Err(error)) => {
                state.done = true;
                return Some((Err(error), state));
            }
            None => state.done = true,
        }
    }
    if state.pending.len() >= DECODER_CHUNK_BYTES {
        let rest = state.pending.split_off(DECODER_CHUNK_BYTES);
        return Some((Ok(mem::replace(&mut state.pending, rest)), state));
    }
    if state.pending.is_empty() {
        return None;
    }
    Some((Ok(mem::take(&mut state.pending)), state))
}

// aiter_bytes(chunk_size=1024) hands the decoder fixed 1024-byte reads, whatever the transport chunked
#[must_use]
pub fn rechunk(body: Bytes) -> Bytes {
    let state = RechunkState {
        body,
        pending: Vec::new(),
        done: false,
    };
    stream::unfold(state, next_chunk).boxed()
}

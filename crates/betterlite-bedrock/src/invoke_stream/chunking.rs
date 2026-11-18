use std::mem;
use std::num::NonZeroUsize;

use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use futures_util::stream;

use crate::agentcore_stream::transport::{Chunks, StreamHead};
use crate::bedrock_http_stream::PendingOpen;

// A caller's stream_chunk_size must not make the chunker hold a whole response body: each buffer stays this small.
const MAX_CHUNK_SIZE: usize = 1024 * 1024;

// aiter_bytes(chunk_size) hands the decoder fixed-size reads whatever the transport chunked; a fake stream never reads it.
pub(super) fn apply(pending: PendingOpen, size: Option<u64>) -> PendingOpen {
    // A size beyond the address space keeps the transport's reads, which is what the chunker's flush would hand over anyway.
    let chunk_size = size
        .and_then(|raw| usize::try_from(raw).ok())
        .and_then(|raw| NonZeroUsize::new(raw.min(MAX_CHUNK_SIZE)));
    let Some(chunk_limit) = chunk_size else {
        return pending;
    };
    let PendingOpen { head, body } = pending;
    PendingOpen {
        head: StreamHead {
            status: head.status,
            headers: head.headers,
            chunks: rechunk(head.chunks, chunk_limit),
        },
        body,
    }
}

fn rechunk(chunks: Chunks, size: NonZeroUsize) -> Chunks {
    stream::unfold(
        RechunkState {
            chunks,
            pending: Vec::new(),
            size,
            done: false,
        },
        next_chunk,
    )
    .boxed()
}

struct RechunkState {
    chunks: Chunks,
    pending: Vec<u8>,
    size: NonZeroUsize,
    done: bool,
}

async fn next_chunk(mut state: RechunkState) -> Option<(Result<Vec<u8>, HttpError>, RechunkState)> {
    while !state.done && state.pending.len() < state.size.get() {
        match state.chunks.next().await {
            Some(Ok(bytes)) => state.pending.extend_from_slice(&bytes),
            Some(Err(error)) => {
                // The read ends the body, so no buffered bytes follow the failure.
                state.done = true;
                state.pending.clear();
                return Some((Err(error), state));
            }
            None => state.done = true,
        }
    }
    if state.pending.len() >= state.size.get() {
        let rest = state.pending.split_off(state.size.get());
        return Some((Ok(mem::replace(&mut state.pending, rest)), state));
    }
    if state.pending.is_empty() {
        return None;
    }
    Some((Ok(mem::take(&mut state.pending)), state))
}

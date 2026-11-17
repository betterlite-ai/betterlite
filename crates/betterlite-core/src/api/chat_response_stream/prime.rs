use betterlite_models::decisions::Json;

use crate::api::chat_response_stream_tools::fields::object;
use crate::api::chat_response_stream_tools::state::StreamProgress;

use super::error::ChatResponseStreamError;
use super::source::{AsyncChunkStream, ChunkStream};

type PrimeResult = Result<(), ChatResponseStreamError>;

// Pull the first chunk before response.created so that event carries the chat completion id
pub fn prime_id<S: ChunkStream>(progress: &mut StreamProgress, source: &mut S) -> PrimeResult {
    if std::mem::replace(&mut progress.response_id_primed, true) {
        return Ok(());
    }
    while !settle(progress, source.next_chunk()?) {}
    Ok(())
}

pub async fn prime_id_async<S: AsyncChunkStream>(
    progress: &mut StreamProgress,
    source: &mut S,
) -> PrimeResult {
    if std::mem::replace(&mut progress.response_id_primed, true) {
        return Ok(());
    }
    loop {
        if settle(progress, source.next_chunk().await?) {
            return Ok(());
        }
    }
}

// True once priming has its chunk or has seen the end; a null chunk is not a provider answer and is skipped
fn settle(progress: &mut StreamProgress, chunk: Option<Json>) -> bool {
    let Some(chunk) = chunk else {
        progress.upstream_exhausted = true;
        return true;
    };
    if matches!(chunk, Json::Null) {
        return false;
    }
    adopt_response_id(progress, &chunk);
    progress.buffered_chunk = Some(chunk);
    true
}

fn adopt_response_id(progress: &mut StreamProgress, chunk: &Json) {
    if progress.cached_response_id.is_some() {
        return;
    }
    let Some(Json::Text(id)) = object(chunk).and_then(|fields| fields.get("id")) else {
        return;
    };
    if !id.is_empty() {
        progress.cached_response_id = Some(id.clone());
    }
}

use serde::Deserialize;

use crate::provider_chunk_decode::chunk::{DecodedChunk, ProviderChunk};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::parse_line;

// LiteLLM's _AlephAlphaStreamData.
#[derive(Deserialize)]
struct AlephAlphaStream {
    completions: Option<Vec<AlephAlphaCompletion>>,
}

#[derive(Deserialize)]
struct AlephAlphaCompletion {
    completion: Option<String>,
}

pub fn handle_aleph_alpha_chunk(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let line = chunk.text()?;
    let data: AlephAlphaStream = parse_line(line, line)?;
    decoded.text = data
        .completions
        .and_then(|completions| completions.into_iter().next())
        .and_then(|first| first.completion)
        .ok_or_else(|| ProviderChunkError::RejectedResponse {
            chunk: line.to_owned(),
        })?;
    decoded.is_finished = true;
    decoded.finish_reason = Some("stop".to_owned());
    Ok(())
}

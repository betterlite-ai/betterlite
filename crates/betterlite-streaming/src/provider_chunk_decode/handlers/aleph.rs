use crate::provider_chunk_decode::chunk::{DecodedChunk, ProviderChunk, at, first, text_at};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::parse_line;

pub fn handle_aleph_alpha_chunk(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let line = chunk.text()?;
    let parsed = parse_line(line)?;
    let completion = at(&parsed, "completions")
        .and_then(first)
        .and_then(|entry| text_at(entry, "completion"))
        .ok_or_else(|| ProviderChunkError::RejectedResponse {
            chunk: line.to_owned(),
        })?;
    completion.clone_into(&mut decoded.text);
    decoded.is_finished = true;
    decoded.finish_reason = Some("stop".to_owned());
    Ok(())
}

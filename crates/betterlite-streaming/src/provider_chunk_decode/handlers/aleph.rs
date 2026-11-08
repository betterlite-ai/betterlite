use crate::provider_chunk_decode::chunk::{DecodedChunk, ProviderChunk, as_array, as_object};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::parse_line;
use crate::stream_chunk_shaping::records::JsonValue;

pub fn handle_aleph_alpha_chunk(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let line = chunk.text()?;
    let parsed = parse_line(line)?;
    let completion = as_object(&parsed)
        .and_then(|fields| fields.get("completions"))
        .and_then(as_array)
        .and_then(|completions| completions.first())
        .and_then(as_object)
        .and_then(|completion| completion.get("completion"))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| ProviderChunkError::RejectedResponse {
            chunk: line.to_owned(),
        })?;
    completion.clone_into(&mut decoded.text);
    decoded.is_finished = true;
    decoded.finish_reason = Some("stop".to_owned());
    Ok(())
}

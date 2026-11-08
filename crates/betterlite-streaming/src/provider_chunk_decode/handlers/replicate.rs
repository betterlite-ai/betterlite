use crate::provider_chunk_decode::chunk::{DecodedChunk, ProviderChunk};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::provider_chunk_decode::handlers::{object_of, rejected_text};
use crate::stream_chunk_shaping::records::JsonValue;

pub fn handle_replicate_chunk(
    chunk: &ProviderChunk,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    let fields = object_of(chunk)?;
    if let Some(output) = fields.get("output") {
        output
            .as_str()
            .ok_or(ProviderChunkError::InvalidField { field: "output" })?
            .clone_into(&mut decoded.text);
    }
    match fields.get("status") {
        Some(status) => {
            if status.as_str() == Some("succeeded") {
                decoded.is_finished = true;
                decoded.finish_reason = Some("stop".to_owned());
            }
        }
        None => {
            if fields
                .get("error")
                .is_some_and(|error| !matches!(error, JsonValue::Null))
            {
                return Err(ProviderChunkError::RejectedResponse {
                    chunk: rejected_text(chunk),
                });
            }
        }
    }
    Ok(())
}

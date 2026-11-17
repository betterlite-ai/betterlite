use std::collections::VecDeque;

use betterlite_core::api::messages_stream_split::ChunkSource;
use betterlite_core::api::messages_stream_split::errors::StreamFailure;
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamChunk};

// The mock's SSE events decode through B005's chunk reader, one chunk per read
pub(super) struct Prefetched {
    pub(super) payloads: VecDeque<Vec<u8>>,
    pub(super) pulls: usize,
}

impl ChunkSource for Prefetched {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pulls = self.pulls.saturating_add(1);
        let Some(payload) = self.payloads.pop_front() else {
            return Ok(None);
        };
        let value: JsonValue =
            serde_json::from_slice(&payload).map_err(|error| StreamFailure::Unclassified {
                text: error.to_string(),
            })?;
        let chunk = ProviderChunk::Json(Box::new(value))
            .openai_chunk()
            .map_err(|error| StreamFailure::Unclassified {
                text: error.to_string(),
            })?;
        Ok(Some(chunk))
    }
}

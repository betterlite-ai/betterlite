use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, EventBoundary};
// ast-grep-ignore: no-long-names
use crate::api::transcription_protocol::RealtimeTranscriptionProtocolError as ProtocolError;
use crate::api::vertex_chirp_protocol::events::ChirpBackendEvent;

use super::error::ChirpAdapterError;

pub fn decode_chirp_backend_event(
    bytes: &[u8],
    limits: ApiLimits,
) -> Result<ChirpBackendEvent, ChirpAdapterError> {
    limits
        .check(ApiBoundariesInput::Event(EventBoundary {
            known_bytes: bytes.len(),
        }))
        .map_err(ProtocolError::from)?;
    serde_json::from_slice(bytes).map_err(ChirpAdapterError::Event)
}

use betterlite_streaming::stream_chunk_shaping::StreamShapeError;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, StreamChunk};
use serde_json::{from_value, to_value};

use super::error::ChatStreamCause;

// response.model_dump() without the fields model_response_creator re-reads from their own arguments.
pub(super) fn chunk_object(chunk: &StreamChunk) -> Result<JsonObject, ChatStreamCause> {
    let serialized = to_value(chunk).map_err(StreamShapeError::from)?;
    let mut object: JsonObject = from_value(serialized).map_err(StreamShapeError::from)?;
    object.remove("usage");
    object.remove("hidden_params");
    Ok(object)
}

use betterlite_streaming::stream_chunk_shaping::StreamShapeError;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, StreamChunk};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{from_value, to_value};

use super::error::ChatStreamCause;

// Python hands these values around as objects; a serde round trip keeps every field the same.
pub(super) fn bridge<T, U>(value: &T) -> Result<U, ChatStreamCause>
where
    T: Serialize + ?Sized,
    U: DeserializeOwned,
{
    let encoded = to_value(value).map_err(StreamShapeError::from)?;
    from_value(encoded).map_err(StreamShapeError::from)
}

// response.model_dump() without the fields model_response_creator re-reads from their own arguments.
pub(super) fn chunk_object(chunk: &StreamChunk) -> Result<JsonObject, ChatStreamCause> {
    let mut object: JsonObject = bridge(chunk)?;
    object.remove("usage");
    object.remove("hidden_params");
    Ok(object)
}

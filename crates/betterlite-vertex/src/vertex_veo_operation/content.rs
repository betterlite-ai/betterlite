use betterlite_prompts::arguments::json::Json;
use betterlite_streaming::stream_content_assembly::decode_stream_audio_fragment;

use super::decode::{VertexVeoOperation, decode_vertex_veo_operation};
use super::error::VeoOperationError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

// transform_video_content_response: only a completed operation's first video becomes bytes, and a gcsUri is never fetched
pub fn content_response(body: &[u8]) -> Result<Vec<u8>, VeoOperationError> {
    let operation = decode_vertex_veo_operation(body)?;
    if !operation.done {
        return Err(VeoOperationError::NotComplete);
    }
    let encoded = encoded_video(&operation)?;
    decode_stream_audio_fragment(encoded, MAX_RESPONSE_BYTES)
        .map_err(VeoOperationError::VideoBase64)
}

fn encoded_video(operation: &VertexVeoOperation) -> Result<&str, VeoOperationError> {
    let fields = match &operation.response {
        None | Some(Json::Null) => return Err(VeoOperationError::NoVideos),
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(VeoOperationError::ResponseShape);
        }
    };
    let items = match fields.get("videos") {
        None | Some(Json::Null) => return Err(VeoOperationError::NoVideos),
        Some(Json::List(items)) => items,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(VeoOperationError::ResponseShape);
        }
    };
    let Some(first) = items.first() else {
        return Err(VeoOperationError::NoVideos);
    };
    let Json::Object(video) = first else {
        return Err(VeoOperationError::ResponseShape);
    };
    let Some(Json::Text(encoded)) = video.get("bytesBase64Encoded") else {
        return Err(VeoOperationError::NoBase64);
    };
    if encoded.is_empty() {
        return Err(VeoOperationError::NoBase64);
    }
    Ok(encoded.as_str())
}

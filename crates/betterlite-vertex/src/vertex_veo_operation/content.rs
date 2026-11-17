use betterlite_streaming::stream_content_assembly::decode_stream_audio_fragment;

use super::decode::decode_vertex_veo_operation;
use super::error::VeoOperationError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

// transform_video_content_response: only a completed operation's first video becomes bytes, and a gcsUri is never fetched
pub fn content_response(body: &[u8]) -> Result<Vec<u8>, VeoOperationError> {
    let operation = decode_vertex_veo_operation(body)?;
    if !operation.done {
        return Err(VeoOperationError::NotComplete);
    }
    let Some(video) = operation.response.videos.first() else {
        return Err(VeoOperationError::NoVideos);
    };
    if video.bytes_base64_encoded.is_empty() {
        return Err(VeoOperationError::NoBase64);
    }
    decode_stream_audio_fragment(&video.bytes_base64_encoded, MAX_RESPONSE_BYTES)
        .map_err(VeoOperationError::VideoBase64)
}

use betterlite_models::decisions::Json;

use super::error::ImageEditError;
use crate::compatible_wire::error::OpenAiLikeError;
use crate::image_generation::decode::decode_image_reply;
use crate::image_generation::reply::ImageGenerationReply;

// only a body that is not JSON is wrapped; a JSON body of the wrong shape keeps the decoder's own error
pub fn transform_image_edit_response(
    status: u16,
    body: &[u8],
    created_seconds: i64,
) -> Result<ImageGenerationReply, ImageEditError> {
    serde_json::from_slice::<Json>(body).map_err(|_invalid| {
        ImageEditError::OpenAi(OpenAiLikeError {
            status_code: status,
            // httpx decodes a body with errors="replace"; this crate names the bytes it cannot read
            message: str::from_utf8(body).map_or_else(
                |_invalid| format!("{} bytes that are not UTF-8", body.len()),
                str::to_owned,
            ),
        })
    })?;
    decode_image_reply(body, created_seconds).map_err(ImageEditError::Reply)
}

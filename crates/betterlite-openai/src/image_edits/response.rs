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
            message: String::from_utf8_lossy(body).into_owned(),
        })
    })?;
    decode_image_reply(body, created_seconds).map_err(ImageEditError::Reply)
}

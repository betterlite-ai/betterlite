use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::image_contracts::records::{ImageData, ImageResponse, ImageUsage};

use super::error::ImagenEditError;
use crate::vertex_imagen_generation::decode::decode_imagen_generation;

// transform_image_edit_response: only a prediction holding the bytesBase64Encoded key becomes an image
// ImageResponse() stamps the current time and a zero usage, and the edit transform leaves both alone
pub fn transform_image_edit_response(
    reply: &UpstreamResponse,
    now_seconds: i64,
) -> Result<ImageResponse, ImagenEditError> {
    let decoded = decode_imagen_generation(reply)?;
    let mut data = Vec::new();
    for b64_json in decoded.images {
        data.push(ImageData {
            b64_json,
            revised_prompt: None,
            url: None,
        });
    }
    Ok(ImageResponse {
        created: now_seconds,
        data: Some(data),
        usage: Some(ImageUsage::default()),
    })
}

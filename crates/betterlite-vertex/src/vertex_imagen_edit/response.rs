use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::image_contracts::records::ImageData;
use serde::Serialize;

use super::error::ImagenEditError;
use crate::vertex_imagen_generation::decode::decode_imagen_generation;

// ImageResponse.data: the edit transform answers one image per prediction and leaves usage alone
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImagenEditResponse {
    pub data: Vec<ImageData>,
}

// transform_image_edit_response: only a prediction holding the bytesBase64Encoded key becomes an image
pub fn transform_image_edit_response(
    reply: &UpstreamResponse,
) -> Result<ImagenEditResponse, ImagenEditError> {
    let decoded = decode_imagen_generation(reply)?;
    let mut data = Vec::new();
    for b64_json in decoded.images {
        data.push(ImageData {
            b64_json,
            revised_prompt: None,
            url: None,
        });
    }
    Ok(ImagenEditResponse { data })
}

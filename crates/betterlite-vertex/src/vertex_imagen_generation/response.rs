use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::image_contracts::records::{ImageData, ImageUsage};
use serde::Serialize;

use super::decode::decode_imagen_generation;
use super::error::ImagenGenerationError;

// transform_image_generation_response only appends to data, so usage stays whatever the caller holds
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImagenGenerationResponse {
    pub data: Vec<ImageData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ImageUsage>,
}

// ast-grep-ignore: no-long-names
pub fn transform_image_generation_response(
    reply: &UpstreamResponse,
    response: &mut ImagenGenerationResponse,
) -> Result<(), ImagenGenerationError> {
    let decoded = decode_imagen_generation(reply)?;
    for prediction in decoded.predictions.unwrap_or_default() {
        // the key's presence alone appends an image, so an empty encoded string still counts
        let Some(encoded) = prediction.bytes_base64_encoded else {
            continue;
        };
        response.data.push(ImageData {
            b64_json: Some(encoded),
            revised_prompt: None,
            url: None,
        });
    }
    Ok(())
}

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::image_contracts::records::ImageData;
use serde::Serialize;

use super::error::GeminiEditError;
use crate::vertex_gemini_images::decode::decode_google_image;

// ImageResponse.data, with neither the usage nor the thought signatures generation keeps
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GeminiEditResponse {
    pub data: Vec<ImageData>,
}

// transform_image_edit_response: only a truthy inlineData.data becomes an edit image
pub fn transform_image_edit_response(
    reply: &UpstreamResponse,
) -> Result<GeminiEditResponse, GeminiEditError> {
    let decoded = decode_google_image(reply)?;
    let mut data = Vec::new();
    for candidate in decoded.candidates.unwrap_or_default() {
        let Some(content) = candidate.content else {
            continue;
        };
        for part in content.parts {
            let Some(encoded) = part
                .inline_data
                .and_then(|inline_data| inline_data.data)
                .filter(|encoded| !encoded.is_empty())
            else {
                continue;
            };
            data.push(ImageData {
                b64_json: Some(encoded),
                revised_prompt: None,
                url: None,
            });
        }
    }
    Ok(GeminiEditResponse { data })
}

use betterlite_http::proxy::UpstreamResponse;

use super::error::StudioImageEditError;
use crate::studio_image_generation::response::ImageGenerationResponse;
use crate::studio_image_generation::usage::{decode_image_usage, transform_gemini_image_usage};
use crate::vertex_gemini_images::decode::{GoogleImageCandidate, decode_google_image};
use crate::vertex_gemini_images::response::GeneratedImage;

// transform_image_edit_response: LiteLLM builds the same ImageResponse generation does, priced by its calculator
pub fn transform_image_edit_response(
    reply: &UpstreamResponse,
) -> Result<ImageGenerationResponse, StudioImageEditError> {
    let decoded = decode_google_image(reply)?;
    let candidates = decoded.candidates.as_deref().unwrap_or_default();
    // "usageMetadata" in the reply decides whether usage exists, even beside no images at all
    let usage = match decoded.usage_metadata.as_deref() {
        Some(raw) => Some(transform_gemini_image_usage(&decode_image_usage(raw)?)?),
        None => None,
    };
    Ok(ImageGenerationResponse {
        data: edit_images(candidates),
        usage,
    })
}

// only a truthy inlineData.data becomes an image, and an edit copies no thought signature
fn edit_images(candidates: &[GoogleImageCandidate]) -> Vec<GeneratedImage> {
    let mut data = Vec::new();
    for candidate in candidates {
        let Some(content) = &candidate.content else {
            continue;
        };
        for part in &content.parts {
            let Some(encoded) = part
                .inline_data
                .as_ref()
                .and_then(|inline_data| inline_data.data.as_ref())
                .filter(|encoded| !encoded.is_empty())
            else {
                continue;
            };
            data.push(GeneratedImage {
                b64_json: Some(encoded.clone()),
                url: None,
                revised_prompt: None,
                provider_specific_fields: None,
            });
        }
    }
    data
}

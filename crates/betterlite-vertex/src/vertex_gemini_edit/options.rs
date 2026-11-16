use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;

use super::error::GeminiEditError;
use crate::vertex_image_identity::size_aspect_ratio;

// get_supported_openai_params: the edit dialect advertises size and nothing else
pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["size"];

// map_openai_params: a present size becomes the aspect ratio the request will carry
pub fn map_openai_params(optional_params: &ImageParams) -> Result<ImageParams, GeminiEditError> {
    let Some(size) = optional_params.get("size") else {
        return Ok(ImageParams::new());
    };
    let size = match size {
        Json::Text(size) => size.as_str(),
        // LiteLLM's `size or ""` sends an absent or null size to the square fallback
        Json::Null => "",
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            return Err(GeminiEditError::FieldType {
                param: "size",
                expected: "a string",
            });
        }
    };
    let mut mapped = ImageParams::new();
    mapped.insert(
        "aspectRatio".to_owned(),
        Json::Text(size_aspect_ratio(size).to_owned()),
    );
    Ok(mapped)
}

use betterlite_models::decisions::{Extra, Json};

use super::error::ImagenGenerationError;
use crate::vertex_image_identity::size_aspect_ratio;

// get_supported_openai_params: the Imagen predict API understands only these two OpenAI options
pub const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["n", "size"];

pub fn map_openai_params(
    non_default_params: &Extra,
    optional_params: &Extra,
) -> Result<Extra, ImagenGenerationError> {
    let mut mapped = Extra::new();
    for (key, value) in non_default_params {
        if optional_params.contains_key(key) || !SUPPORTED_OPENAI_PARAMS.contains(&key.as_str()) {
            continue;
        }
        match key.as_str() {
            "n" => {
                mapped.insert("sampleCount".to_owned(), value.clone());
            }
            "size" => {
                let Json::Text(size) = value else {
                    return Err(ImagenGenerationError::FieldType {
                        param: "size",
                        expected: "a string",
                    });
                };
                let ratio = size_aspect_ratio(size);
                mapped.insert("aspectRatio".to_owned(), Json::Text(ratio.to_owned()));
            }
            // LiteLLM passes every other supported option through untouched, and n and size are the whole list
            _ => {}
        }
    }
    Ok(mapped)
}

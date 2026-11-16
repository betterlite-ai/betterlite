use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;

use super::error::ImagenEditError;
use crate::vertex_image_identity::size_aspect_ratio;

// get_supported_openai_params: the edit dialect advertises n, size and mask
pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["n", "size", "mask"];

// map_openai_params: n becomes sampleCount and size an aspect ratio, which the request never carries
pub fn map_openai_params(optional_params: &ImageParams) -> Result<ImageParams, ImagenEditError> {
    let mut mapped = ImageParams::new();
    if let Some(value) = optional_params.get("n")
        && let Some(number) = sample_count(value)?
    {
        mapped.insert("sampleCount".to_owned(), number);
    }
    if let Some(value) = optional_params.get("size") {
        mapped.insert(
            "aspectRatio".to_owned(),
            Json::Text(aspect_ratio(value)?.to_owned()),
        );
    }
    // the mask is a file record rather than JSON, so it travels to the transform as its own argument
    Ok(mapped)
}

// LiteLLM copies whatever n held, and a null one takes the transform's default of one sample
fn sample_count(value: &Json) -> Result<Option<Json>, ImagenEditError> {
    match value {
        Json::Null => Ok(None),
        Json::Number(number) if number.is_i64() || number.is_u64() => {
            Ok(Some(Json::Number(number.clone())))
        }
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(ImagenEditError::FieldType {
                param: "n",
                expected: "an integer",
            })
        }
    }
}

// _map_size_to_aspect_ratio: five ratios and a square fallback for anything else
fn aspect_ratio(value: &Json) -> Result<&'static str, ImagenEditError> {
    match value {
        Json::Text(text) => Ok(size_aspect_ratio(text)),
        // LiteLLM's map lookup sends a null size to the square pair
        Json::Null => Ok("1:1"),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(ImagenEditError::FieldType {
                param: "size",
                expected: "a string",
            })
        }
    }
}

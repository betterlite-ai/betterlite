use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;

use crate::api::fal::images::error::FalImageError;

pub const GENERIC_OPENAI_PARAMS: [&str; 3] = ["n", "response_format", "size"];

#[must_use]
pub fn image_generation_body(prompt: &str, optional_params: ImageParams) -> ImageParams {
    let mut body = ImageParams::new();
    body.insert("prompt".to_owned(), Json::Text(prompt.to_owned()));
    for (name, value) in optional_params {
        body.insert(name, value);
    }
    body
}

pub fn map_openai_params(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
) -> Result<ImageParams, FalImageError> {
    let mut mapped = optional_params;
    for (name, value) in non_default_params {
        if mapped.contains_key(name) {
            continue;
        }
        if GENERIC_OPENAI_PARAMS.contains(&name.as_str()) {
            mapped.insert(name.clone(), value.clone());
        } else if !drop_params {
            return Err(FalImageError::UnsupportedParameters {
                parameters: vec![name.clone()],
                model: model.to_owned(),
            });
        }
    }
    Ok(mapped)
}

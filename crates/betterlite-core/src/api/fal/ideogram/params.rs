use betterlite_openai::api::image_contracts::records::ImageParams;

use crate::api::fal::ideogram::error::IdeogramError;
use crate::api::fal::ideogram::size::map_image_size;
use crate::api::fal::images::params::GENERIC_OPENAI_PARAMS;

pub fn map_openai_params(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
) -> Result<ImageParams, IdeogramError> {
    let mut mapped = optional_params;
    for (name, value) in non_default_params {
        // LiteLLM tests the OpenAI name, so optional_params carrying it wins over the mapping
        if mapped.contains_key(name) {
            continue;
        }
        if !GENERIC_OPENAI_PARAMS.contains(&name.as_str()) {
            if drop_params {
                continue;
            }
            return Err(IdeogramError::UnsupportedParameters {
                parameters: vec![name.clone()],
                model: model.to_owned(),
            });
        }
        match name.as_str() {
            "n" => {
                mapped.insert("num_images".to_owned(), value.clone());
            }
            "size" => {
                mapped.insert("image_size".to_owned(), map_image_size(value));
            }
            _ => {}
        }
    }
    Ok(mapped)
}

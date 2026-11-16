use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;

use crate::api::fal::stable_diffusion::error::StableDiffusionError;
use crate::api::fal::stable_diffusion::size::map_image_size;

pub const PARAM_TRANSLATION: [(&str, &str); 3] = [
    ("n", "num_images"),
    ("response_format", "output_format"),
    ("size", "image_size"),
];

pub fn map_openai_params(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
) -> Result<ImageParams, StableDiffusionError> {
    let mut mapped = optional_params;
    for (name, value) in non_default_params {
        // LiteLLM tests the OpenAI name, so optional_params carrying it wins over the mapping
        if mapped.contains_key(name) {
            continue;
        }
        let Some((_, destination)) = PARAM_TRANSLATION
            .iter()
            .find(|(source, _)| *source == name.as_str())
        else {
            if drop_params {
                continue;
            }
            return Err(StableDiffusionError::UnsupportedParameters {
                parameters: vec![name.clone()],
                model: model.to_owned(),
            });
        };
        mapped.insert((*destination).to_owned(), translated_value(name, value)?);
    }
    Ok(mapped)
}

fn translated_value(name: &str, value: &Json) -> Result<Json, StableDiffusionError> {
    match name {
        "size" => map_image_size(value),
        "response_format" => Ok(response_format(value)),
        _ => Ok(value.clone()),
    }
}

fn response_format(value: &Json) -> Json {
    match value {
        Json::Text(text) if text == "url" || text == "b64_json" => Json::Text("jpeg".to_owned()),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => value.clone(),
    }
}

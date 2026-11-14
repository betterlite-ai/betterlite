use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;

use crate::api::fal::gpt_image::GPT_OPENAI_PARAMS;
use crate::api::fal::gpt_image::PARAM_TRANSLATION;
use crate::api::fal::gpt_image::quality::map_gpt_image_quality;
use crate::api::fal::gpt_image::size::map_gpt_image_size;
use crate::api::fal::images::error::FalImageError;

pub fn map_openai_params(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
    catalog: &ModelCatalog,
) -> Result<ImageParams, FalImageError> {
    let parameters: Vec<String> = non_default_params
        .keys()
        .filter(|name| {
            !GPT_OPENAI_PARAMS.contains(&name.as_str()) && !optional_params.contains_key(*name)
        })
        .cloned()
        .collect();
    if !parameters.is_empty() && !drop_params {
        return Err(FalImageError::UnsupportedParameters {
            parameters,
            model: model.to_owned(),
        });
    }
    let mut mapped = optional_params;
    for (name, value) in non_default_params {
        let Some((_, destination)) = PARAM_TRANSLATION
            .iter()
            .find(|(source, _)| *source == name.as_str())
        else {
            continue;
        };
        if mapped.contains_key(*destination) {
            continue;
        }
        mapped.insert(
            (*destination).to_owned(),
            translate_value(name, value, model, catalog),
        );
    }
    Ok(mapped)
}

fn translate_value(name: &str, value: &Json, model: &str, catalog: &ModelCatalog) -> Json {
    match name {
        "size" => map_gpt_image_size(value),
        "quality" => map_gpt_image_quality(value, model, catalog),
        _ => value.clone(),
    }
}

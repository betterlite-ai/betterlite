use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;

use crate::api::fal::gpt_image::quality::map_gpt_image_quality;
use crate::api::fal::gpt_image::size::map_gpt_image_size;

pub const GENERIC_OPENAI_PARAMS: [&str; 5] = ["background", "mask", "n", "quality", "size"];
pub const PARAM_TRANSLATION: [(&str, &str); 4] = [
    ("background", "background"),
    ("n", "num_images"),
    ("quality", "quality"),
    ("size", "image_size"),
];

#[must_use]
pub fn map_openai_params(
    options: &ImageParams,
    model: &str,
    catalog: &ModelCatalog,
) -> ImageParams {
    let mut mapped = ImageParams::new();
    for (name, value) in options {
        if matches!(value, Json::Null) {
            continue;
        }
        let destination = PARAM_TRANSLATION
            .iter()
            .find_map(|(source, destination)| (*source == name.as_str()).then_some(*destination))
            .unwrap_or(name.as_str());
        mapped.insert(
            destination.to_owned(),
            translate_value(name, value, model, catalog),
        );
    }
    mapped
}

fn translate_value(name: &str, value: &Json, model: &str, catalog: &ModelCatalog) -> Json {
    match name {
        "size" => map_gpt_image_size(value),
        "quality" => map_gpt_image_quality(value, model, catalog),
        _ => value.clone(),
    }
}

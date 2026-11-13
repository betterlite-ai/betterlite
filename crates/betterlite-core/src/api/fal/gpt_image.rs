pub mod params;
pub mod quality;
pub mod size;
mod tests;

use crate::api::fal::images::settings::FalImageSettings;

pub const MODEL_PREFIX: &str = "openai/";

pub const GPT_OPENAI_PARAMS: [&str; 5] =
    ["n", "output_format", "quality", "response_format", "size"];

pub const PARAM_TRANSLATION: [(&str, &str); 4] = [
    ("n", "num_images"),
    ("size", "image_size"),
    ("quality", "quality"),
    ("output_format", "output_format"),
];

#[must_use]
pub fn complete_url(settings: &FalImageSettings, api_base: Option<&str>, model: &str) -> String {
    let endpoint = if model.starts_with(MODEL_PREFIX) {
        model.to_owned()
    } else {
        format!("{MODEL_PREFIX}{model}")
    };
    settings.complete_url(api_base, &endpoint)
}

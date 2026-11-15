pub mod error;
pub mod params;
pub mod reply;
pub mod size;
mod tests;

use crate::api::fal::images::settings::FalImageSettings;

pub const FAL_AI_PREFIX: &str = "fal-ai/";

#[must_use]
pub fn complete_url(settings: &FalImageSettings, api_base: Option<&str>, model: &str) -> String {
    let endpoint = if model.starts_with(FAL_AI_PREFIX) || model.contains('/') {
        model.to_owned()
    } else {
        format!("{FAL_AI_PREFIX}{model}")
    };
    settings.complete_url(api_base, &endpoint)
}

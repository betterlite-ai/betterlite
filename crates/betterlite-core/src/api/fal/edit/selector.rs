use crate::api::fal::edit::{flux_lora_depth, transformation};
use crate::api::fal::images::settings::FalImageSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Config {
    Generic,
    FluxLoraDepth,
}

impl Config {
    #[must_use]
    pub fn complete_url(
        self,
        settings: &FalImageSettings,
        api_base: Option<&str>,
        model: &str,
    ) -> String {
        match self {
            Self::Generic => transformation::complete_url(settings, api_base, model),
            Self::FluxLoraDepth => settings.complete_url(api_base, flux_lora_depth::DEPTH_ENDPOINT),
        }
    }
}

pub const DEPTH_MARKER: &str = "flux-lora-depth";

#[must_use]
pub fn for_model(model: &str) -> Config {
    if model.to_lowercase().contains(DEPTH_MARKER) {
        Config::FluxLoraDepth
    } else {
        Config::Generic
    }
}

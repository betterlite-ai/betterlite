use betterlite_config::loader::EnvironmentLookup;

use crate::studio_image_generation::settings::Settings;

// the LiteLLM class the edit package factory answers with, whatever model it is asked about
pub const GEMINI_IMAGE_EDIT_CONFIG: &str = "GeminiImageEditConfig";

#[derive(Clone, Debug, Default)]
pub struct GeminiImageEditConfig {
    pub settings: Settings,
}

#[must_use]
pub fn get_gemini_image_edit_config(
    _model: &str,
    lookup: &dyn EnvironmentLookup,
) -> GeminiImageEditConfig {
    GeminiImageEditConfig {
        settings: Settings::from_environment(lookup),
    }
}

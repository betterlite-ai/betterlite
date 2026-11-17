use betterlite_config::loader::EnvironmentLookup;

use super::settings::Settings;

// the LiteLLM class this package factory answers with, whatever model it is asked about
pub const GOOGLE_IMAGE_GEN_CONFIG: &str = "GoogleImageGenConfig";

#[derive(Clone, Debug, Default)]
pub struct GoogleImageGenerationConfig {
    pub settings: Settings,
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn get_gemini_image_generation_config(
    _model: &str,
    lookup: &dyn EnvironmentLookup,
) -> GoogleImageGenerationConfig {
    GoogleImageGenerationConfig {
        settings: Settings::from_environment(lookup),
    }
}

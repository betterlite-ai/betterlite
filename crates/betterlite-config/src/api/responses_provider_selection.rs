use crate::sections::openai_like_settings::Settings;
use crate::types::Config;

mod tests;

// B002's loader already resolves both OpenAI-like environment variables the compatible fallback reads.
#[must_use]
pub fn compatible_provider_settings(config: &Config) -> &Settings {
    &config.sections.openai_like_settings
}

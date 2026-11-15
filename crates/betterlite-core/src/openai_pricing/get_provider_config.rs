use betterlite_openai::openai_base_config::{ConfigFamily, Headers, OpenAiConfig, OpenAiSettings};

mod tests;

// OpenAIConfig() takes no arguments, so the model never changes the configuration the passthrough handler reads
#[must_use]
pub fn get_provider_config(_model: &str) -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::OpenAi,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

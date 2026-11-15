use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::types::RoutingStrategy;
use secrecy::SecretString;

// LiteLLM's Recraft generation and edit configurations read the same two settings from the environment
#[derive(Debug, Clone)]
pub struct RecraftImageSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl RecraftImageSettings {
    #[must_use]
    pub fn from_environment(lookup: &dyn EnvironmentLookup) -> Self {
        let (api_key, api_base) = RoutingStrategy::recraft_image_settings(lookup);
        Self { api_key, api_base }
    }
}

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;
use crate::sections::Section;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellmSettings {
    inception_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub gradient_ai_agent_endpoint: Option<String>,
    pub gradient_ai_api_key: Option<SecretString>,
    pub heroku_api_base: Option<String>,
    pub heroku_api_key: Option<SecretString>,
    pub hyperbolic_api_base: Option<String>,
    pub hyperbolic_api_key: Option<SecretString>,
    pub lambda_api_base: Option<String>,
    pub lambda_api_key: Option<SecretString>,
    pub inception_api_base: Option<String>,
    pub inception_api_key: Option<SecretString>,
    pub inception_key: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM's get_secret_str returns an empty environment value as-is, and Gradient rejects only a missing key
        Ok(Settings {
            gradient_ai_agent_endpoint: lookup.lookup("GRADIENT_AI_AGENT_ENDPOINT"),
            gradient_ai_api_key: lookup.lookup("GRADIENT_AI_API_KEY").map(SecretString::from),
            heroku_api_base: lookup.lookup("HEROKU_API_BASE"),
            heroku_api_key: lookup.lookup("HEROKU_API_KEY").map(SecretString::from),
            hyperbolic_api_base: lookup.lookup("HYPERBOLIC_API_BASE"),
            hyperbolic_api_key: lookup.lookup("HYPERBOLIC_API_KEY").map(SecretString::from),
            lambda_api_base: lookup.lookup("LAMBDA_API_BASE"),
            lambda_api_key: lookup.lookup("LAMBDA_API_KEY").map(SecretString::from),
            inception_api_base: lookup.lookup("INCEPTION_API_BASE"),
            inception_api_key: lookup.lookup("INCEPTION_API_KEY").map(SecretString::from),
            inception_key: resolve_secret(
                self.litellm_settings.inception_key,
                "litellm_settings.inception_key",
                lookup,
            )?,
        })
    }
}

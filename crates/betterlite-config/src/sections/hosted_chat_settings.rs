use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use secrecy::SecretString;
use serde::Deserialize;
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlmSettings,
}
#[derive(Debug, Default, Deserialize)]
struct RawLiteLlmSettings {
    drop_params: Option<bool>,
}
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub featherless_ai_api_base: Option<String>,
    pub featherless_api_base: Option<String>,
    pub featherless_ai_api_key: Option<SecretString>,
    pub featherless_api_key: Option<SecretString>,
    pub aiml_api_base: Option<String>,
    pub aiml_api_key: Option<SecretString>,
    pub drop_params: bool,
}
impl Section for Raw {
    type Settings = Settings;
    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // the loader normalizes LITELLM_DROP_PARAMS, turning an invalid value on with a warning
        let environment_drop = betterlite_env::flag(
            "LITELLM_DROP_PARAMS",
            &lookup.lookup("LITELLM_DROP_PARAMS").unwrap_or_default(),
        )?;
        Ok(Settings {
            featherless_ai_api_base: value(lookup, "FEATHERLESS_AI_API_BASE"),
            featherless_api_base: value(lookup, "FEATHERLESS_API_BASE"),
            featherless_ai_api_key: value(lookup, "FEATHERLESS_AI_API_KEY").map(SecretString::from),
            featherless_api_key: value(lookup, "FEATHERLESS_API_KEY").map(SecretString::from),
            aiml_api_base: value(lookup, "AIML_API_BASE"),
            aiml_api_key: value(lookup, "AIML_API_KEY").map(SecretString::from),
            drop_params: self
                .litellm_settings
                .drop_params
                .or(environment_drop)
                .unwrap_or(false),
        })
    }
}
// LiteLLM reads each alias with `or`, so an empty environment value falls through like an absent one
fn value<L: EnvironmentLookup>(lookup: &L, name: &str) -> Option<String> {
    match lookup.lookup(name) {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

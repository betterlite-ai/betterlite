use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::providers::hosted_chat::environment;
use crate::sections::Section;

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
        let values = environment::resolve(lookup);
        // the loader normalizes LITELLM_DROP_PARAMS, turning an invalid value on with a warning
        let environment_drop = betterlite_env::flag(
            "LITELLM_DROP_PARAMS",
            &lookup.lookup("LITELLM_DROP_PARAMS").unwrap_or_default(),
        )?;
        Ok(Settings {
            featherless_ai_api_base: values.featherless_ai_api_base,
            featherless_api_base: values.featherless_api_base,
            featherless_ai_api_key: values.featherless_ai_api_key,
            featherless_api_key: values.featherless_api_key,
            aiml_api_base: values.aiml_api_base,
            aiml_api_key: values.aiml_api_key,
            drop_params: self
                .litellm_settings
                .drop_params
                .or(environment_drop)
                .unwrap_or(false),
        })
    }
}

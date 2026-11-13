use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

pub const DEFAULT_API_BASE: &str = "https://api.deepinfra.com/v1/openai";
pub const DEFAULT_MINIMUM_TEMPERATURE: f64 = 0.0001;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlmSettings {
    drop_params: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub min_non_zero_temperature: f64,
    pub drop_params: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_base: None,
            api_key: None,
            min_non_zero_temperature: DEFAULT_MINIMUM_TEMPERATURE,
            drop_params: false,
        }
    }
}

impl Settings {
    // LiteLLM chains the explicit value, the environment alias and the default behind Python's `or`, so an empty one falls through
    #[must_use]
    pub fn resolve(
        &self,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> (String, Option<SecretString>) {
        let base = api_base
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| self.api_base.clone())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let key = api_key
            .filter(|value| !value.expose_secret().is_empty())
            .cloned()
            .or_else(|| self.api_key.clone());
        (base, key)
    }

    // the rerank config only consults the environment key when the caller supplied none, and an empty caller key is kept
    #[must_use]
    pub fn rerank_key(&self, api_key: Option<&SecretString>) -> Option<SecretString> {
        api_key.or(self.api_key.as_ref()).cloned()
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let text = lookup
            .lookup("MIN_NON_ZERO_TEMPERATURE")
            .unwrap_or_default();
        let minimum = betterlite_env::number::<f64>("MIN_NON_ZERO_TEMPERATURE", &text)?
            .unwrap_or(DEFAULT_MINIMUM_TEMPERATURE);
        let environment_drop = betterlite_env::flag(
            "LITELLM_DROP_PARAMS",
            &lookup.lookup("LITELLM_DROP_PARAMS").unwrap_or_default(),
        )?;
        Ok(Settings {
            api_base: lookup
                .lookup("DEEPINFRA_API_BASE")
                .filter(|value| !value.is_empty()),
            api_key: lookup
                .lookup("DEEPINFRA_API_KEY")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            min_non_zero_temperature: minimum,
            drop_params: self
                .litellm_settings
                .drop_params
                .or(environment_drop)
                .unwrap_or(false),
        })
    }
}

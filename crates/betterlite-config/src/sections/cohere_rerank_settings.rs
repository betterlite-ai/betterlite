use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    cohere_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub cohere_api_key: Option<SecretString>,
    pub co_api_key: Option<SecretString>,
    pub cohere_key: Option<SecretString>,
}

impl Settings {
    #[must_use]
    pub fn resolve_api_key(&self, explicit: Option<&SecretString>) -> Option<SecretString> {
        if let Some(key) = explicit {
            return Some(key.clone());
        }
        let environment = [&self.cohere_api_key, &self.co_api_key]
            .into_iter()
            .flatten()
            .find(|key| !key.expose_secret().is_empty());
        environment.or(self.cohere_key.as_ref()).cloned()
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            cohere_api_key: configured(lookup.lookup("COHERE_API_KEY")),
            co_api_key: configured(lookup.lookup("CO_API_KEY")),
            cohere_key: resolve_secret(
                self.litellm_settings.cohere_key,
                "litellm_settings.cohere_key",
                lookup,
            )?,
        })
    }
}

fn configured(value: Option<String>) -> Option<SecretString> {
    match value {
        Some(text) if !text.is_empty() => Some(SecretString::from(text)),
        Some(_) | None => None,
    }
}

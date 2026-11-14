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
    infinity_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub infinity_api_key: Option<SecretString>,
    pub infinity_key: Option<SecretString>,
}

impl Settings {
    // an empty environment value falls through to litellm_settings.infinity_key, which is kept even when empty
    #[must_use]
    pub fn resolve_api_key(&self, explicit: Option<&SecretString>) -> Option<SecretString> {
        let environment = self
            .infinity_api_key
            .as_ref()
            .filter(|key| !key.expose_secret().is_empty());
        explicit
            .or(environment)
            .or(self.infinity_key.as_ref())
            .cloned()
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment = lookup
            .lookup("INFINITY_API_KEY")
            .filter(|key| !key.is_empty());
        Ok(Settings {
            infinity_api_key: environment.map(SecretString::from),
            infinity_key: resolve_secret(
                self.litellm_settings.infinity_key,
                "litellm_settings.infinity_key",
                lookup,
            )?,
        })
    }
}

mod tests;

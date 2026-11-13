use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    lemonade_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub lemonade_key: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // an empty environment variable counts as unset
        let read = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Ok(Settings {
            api_base: read("LEMONADE_API_BASE"),
            api_key: read("LEMONADE_API_KEY").map(SecretString::from),
            lemonade_key: resolve_secret(
                self.litellm_settings.lemonade_key,
                "litellm_settings.lemonade_key",
                lookup,
            )?,
        })
    }
}

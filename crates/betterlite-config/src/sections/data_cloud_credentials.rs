use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;
use crate::sections::Section;

#[path = "data_cloud_credentials/tests.rs"]
mod tests;

// LitellmSettings' databricks_key, which LiteLLM sets as a module global next to api_key
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellmSettings {
    databricks_key: Option<SecretString>,
}

// Boxed: Sections holds each Settings by value, and config snapshots live inside futures
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub inner: Box<Inner>,
}

#[derive(Debug, Clone, Default)]
pub struct Inner {
    pub databricks_key: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw = self.litellm_settings;
        Ok(Settings {
            inner: Box::new(Inner {
                databricks_key: resolve_secret(
                    raw.databricks_key,
                    "litellm_settings.databricks_key",
                    lookup,
                )?,
            }),
        })
    }
}

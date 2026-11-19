use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;
use crate::sections::Section;

#[path = "enterprise_ml_credentials/tests.rs"]
mod tests;

// LitellmSettings' sap_service_key, the fallback LiteLLM's SAP credentials read after a per-request key
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellmSettings {
    sap_service_key: Option<SecretString>,
}

// Boxed: Sections holds each Settings by value, and config snapshots live inside futures
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub inner: Box<Inner>,
}

#[derive(Debug, Clone, Default)]
pub struct Inner {
    pub sap_service_key: Option<SecretString>,
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
                sap_service_key: resolve_secret(
                    raw.sap_service_key,
                    "litellm_settings.sap_service_key",
                    lookup,
                )?,
            }),
        })
    }
}

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;
use crate::sections::Section;

#[path = "model_hub_credentials/tests.rs"]
mod tests;

// LitellmSettings' huggingface_key, replicate_key, predibase_key and predibase_tenant_id, which LiteLLM sets as module globals
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellmSettings {
    huggingface_key: Option<SecretString>,
    replicate_key: Option<SecretString>,
    predibase_key: Option<SecretString>,
    predibase_tenant_id: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub huggingface_key: Option<SecretString>,
    pub replicate_key: Option<SecretString>,
    pub predibase_key: Option<SecretString>,
    pub predibase_tenant_id: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw = self.litellm_settings;
        Ok(Settings {
            huggingface_key: resolve_secret(
                raw.huggingface_key,
                "litellm_settings.huggingface_key",
                lookup,
            )?,
            replicate_key: resolve_secret(
                raw.replicate_key,
                "litellm_settings.replicate_key",
                lookup,
            )?,
            predibase_key: resolve_secret(
                raw.predibase_key,
                "litellm_settings.predibase_key",
                lookup,
            )?,
            predibase_tenant_id: resolve_secret(
                raw.predibase_tenant_id,
                "litellm_settings.predibase_tenant_id",
                lookup,
            )?,
        })
    }
}

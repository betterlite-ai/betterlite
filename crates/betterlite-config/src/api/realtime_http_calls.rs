mod tests;

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::env_integer::timeout::{RequestTimeout, TimeoutError};
use crate::loader::{ConfigError, ConfigLoader, EnvironmentLookup};

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

// The litellm globals and environment values the realtime HTTP configs read, with the SDK's REQUEST_TIMEOUT.
#[derive(Debug, Clone)]
pub struct RealtimeHttpCallSettings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub openai_api_base: Option<String>,
    pub openai_api_key: Option<SecretString>,
    pub azure_api_base: Option<String>,
    pub azure_api_key: Option<SecretString>,
    pub azure_api_version: Option<String>,
    pub request_timeout_seconds: f64,
}

// Loads the whole config, so an unsupported key fails here exactly as it does for every other section.
pub fn load_settings<L>(source: &str, lookup: L) -> Result<RealtimeHttpCallSettings, SettingsError>
where
    L: EnvironmentLookup,
{
    let timeout = RequestTimeout::resolve(&lookup)?;
    let config = ConfigLoader { lookup }.load(source)?;
    let resource = &config.sections.resource_sdk_settings;
    Ok(RealtimeHttpCallSettings {
        api_base: non_empty_text(resource.api_base.as_deref()),
        api_key: non_empty_secret(resource.api_key.as_ref()),
        openai_key: non_empty_secret(resource.openai_key.as_ref()),
        openai_api_base: non_empty_text(resource.openai_api_base.as_deref()),
        openai_api_key: non_empty_secret(resource.openai_api_key.as_ref()),
        azure_api_base: non_empty_text(resource.azure_api_base.as_deref()),
        azure_api_key: non_empty_secret(resource.azure_api_key.as_ref()),
        azure_api_version: non_empty_text(resource.azure_api_version.as_deref()),
        request_timeout_seconds: timeout.seconds,
    })
}

fn non_empty_text(value: Option<&str>) -> Option<String> {
    let text = value?;
    (!text.is_empty()).then(|| text.to_owned())
}

fn non_empty_secret(value: Option<&SecretString>) -> Option<SecretString> {
    value
        .filter(|secret| !secret.expose_secret().is_empty())
        .cloned()
}

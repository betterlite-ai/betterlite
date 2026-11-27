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
        api_base: resource.api_base.clone().filter(|text| !text.is_empty()),
        api_key: non_empty_secret(resource.api_key.as_ref()),
        openai_key: non_empty_secret(resource.openai_key.as_ref()),
        openai_api_base: resource
            .openai_api_base
            .clone()
            .filter(|text| !text.is_empty()),
        openai_api_key: non_empty_secret(resource.openai_api_key.as_ref()),
        azure_api_base: resource
            .azure_api_base
            .clone()
            .filter(|text| !text.is_empty()),
        azure_api_key: non_empty_secret(resource.azure_api_key.as_ref()),
        azure_api_version: resource
            .azure_api_version
            .clone()
            .filter(|text| !text.is_empty()),
        request_timeout_seconds: timeout.seconds,
    })
}

fn non_empty_secret(value: Option<&SecretString>) -> Option<SecretString> {
    value
        .filter(|secret| !secret.expose_secret().is_empty())
        .cloned()
}

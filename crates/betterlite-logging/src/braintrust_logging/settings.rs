use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

const DEFAULT_API_BASE: &str = "https://api.braintrustdata.com/v1";
const API_KEY_VARIABLE: &str = "BRAINTRUST_API_KEY";
const API_BASE_VARIABLE: &str = "BRAINTRUST_API_BASE";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("{API_KEY_VARIABLE} is not set")]
    MissingApiKey,
}

#[derive(Debug)]
pub struct Settings {
    pub api_key: SecretString,
    pub api_base: String,
    pub default_tags: Vec<String>,
}

impl Settings {
    pub fn resolve(
        api_key: Option<SecretString>,
        api_base: Option<String>,
        default_tags: Vec<String>,
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, SettingsError> {
        let explicit = api_key.filter(|key| !key.expose_secret().is_empty());
        let secret = explicit
            .or_else(|| {
                environment
                    .lookup(API_KEY_VARIABLE)
                    .filter(|value| !value.is_empty())
                    .map(SecretString::from)
            })
            .ok_or(SettingsError::MissingApiKey)?;
        let base = api_base
            .or_else(|| {
                environment
                    .lookup(API_BASE_VARIABLE)
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        Ok(Self {
            api_key: secret,
            api_base: base,
            default_tags,
        })
    }
}

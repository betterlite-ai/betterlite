use std::collections::BTreeMap;

use secrecy::SecretString;
use thiserror::Error;

#[derive(Debug)]
pub struct WorkerSettings {
    pub litellm_url: String,
    pub token: SecretString,
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("the environment variable {name} is required")]
    Missing { name: &'static str },
}

impl WorkerSettings {
    pub fn resolve(environment: &BTreeMap<String, String>) -> Result<Self, SettingsError> {
        let litellm_url = environment
            .get("LITELLM_URL")
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or(SettingsError::Missing {
                name: "LITELLM_URL",
            })?;
        let token = environment
            .get("LENS_WORKER_TOKEN")
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or(SettingsError::Missing {
                name: "LENS_WORKER_TOKEN",
            })?;
        Ok(Self {
            litellm_url: litellm_url.trim_end_matches('/').to_owned(),
            token: SecretString::from(token),
        })
    }
}

#[path = "settings/tests.rs"]
mod tests;

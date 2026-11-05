use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

use crate::api::runway::tasks::error::RunwayTaskError;

const DEFAULT_API_BASE: &str = "https://api.dev.runwayml.com";
const DEFAULT_API_VERSION: &str = "2024-11-06";
const DEFAULT_TIMEOUT_SECONDS: u64 = 600;

#[derive(Debug, Clone)]
pub struct RunwaySettings {
    pub api_base: String,
    pub api_secret: Option<SecretString>,
    pub api_key: Option<SecretString>,
    pub api_version: String,
    pub polling_timeout_seconds: u64,
}

impl RunwaySettings {
    pub fn load(lookup: &dyn EnvironmentLookup) -> Result<Self, RunwayTaskError> {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        let polling_timeout_seconds = match present("RUNWAYML_POLLING_TIMEOUT") {
            Some(text) => betterlite_env::number("RUNWAYML_POLLING_TIMEOUT", &text)?
                .unwrap_or(DEFAULT_TIMEOUT_SECONDS),
            None => DEFAULT_TIMEOUT_SECONDS,
        };
        Ok(Self {
            api_base: present("RUNWAYML_API_BASE").unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
            api_secret: present("RUNWAYML_API_SECRET").map(SecretString::from),
            api_key: present("RUNWAYML_API_KEY").map(SecretString::from),
            api_version: present("RUNWAYML_DEFAULT_API_VERSION")
                .unwrap_or_else(|| DEFAULT_API_VERSION.to_owned()),
            polling_timeout_seconds,
        })
    }
}

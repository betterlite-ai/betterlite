use std::collections::BTreeMap;

use secrecy::SecretString;

pub const API_KEY_VARIABLE: &str = "HUMANLOOP_API_KEY";
pub const CACHE_TTL_VARIABLE: &str = "HUMANLOOP_PROMPT_CACHE_TTL_SECONDS";
pub const DEFAULT_CACHE_TTL_SECONDS: u64 = 60;

#[derive(Debug)]
pub struct HumanloopSettings {
    pub api_key: Option<SecretString>,
    pub cache_ttl_seconds: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error(transparent)]
    CacheTtl(#[from] betterlite_env::EnvError),
}

impl HumanloopSettings {
    pub fn from_environment(environment: &BTreeMap<String, String>) -> Result<Self, SettingsError> {
        let resolved_api_key = environment
            .get(API_KEY_VARIABLE)
            .map(String::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(SecretString::from);
        let cache_ttl_seconds = environment
            .get(CACHE_TTL_VARIABLE)
            .map(|text| betterlite_env::number::<u64>(CACHE_TTL_VARIABLE, text))
            .transpose()?
            .flatten()
            .unwrap_or(DEFAULT_CACHE_TTL_SECONDS);
        Ok(Self {
            api_key: resolved_api_key,
            cache_ttl_seconds,
        })
    }
}

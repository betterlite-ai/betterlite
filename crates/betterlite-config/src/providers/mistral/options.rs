use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};

pub(crate) const DEFAULT_MAX_RECURSE_DEPTH: i64 = 100;

const MAX_DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH";

#[derive(Debug, Clone)]
pub(crate) struct EnvironmentOptions {
    pub(crate) azure_api_base: Option<String>,
    pub(crate) azure_api_key: Option<SecretString>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) max_recurse_depth: i64,
}

pub(crate) fn resolve(lookup: &dyn EnvironmentLookup) -> Result<EnvironmentOptions, ConfigError> {
    let max_recurse_depth = match lookup.lookup(MAX_DEPTH_VARIABLE) {
        None => DEFAULT_MAX_RECURSE_DEPTH,
        Some(text) => betterlite_env::number::<i64>(MAX_DEPTH_VARIABLE, &text)?
            .unwrap_or(DEFAULT_MAX_RECURSE_DEPTH),
    };
    Ok(EnvironmentOptions {
        azure_api_base: lookup
            .lookup("MISTRAL_AZURE_API_BASE")
            .filter(|value| !value.is_empty()),
        azure_api_key: lookup
            .lookup("MISTRAL_AZURE_API_KEY")
            .filter(|value| !value.is_empty())
            .map(SecretString::from),
        api_key: lookup
            .lookup("MISTRAL_API_KEY")
            .filter(|value| !value.is_empty())
            .map(SecretString::from),
        max_recurse_depth,
    })
}

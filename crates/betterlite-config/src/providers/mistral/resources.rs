use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::providers::mistral::options;

const API_BASE_VARIABLE: &str = "MISTRAL_API_BASE";

#[derive(Debug, Clone)]
pub(crate) struct ResourceEnvironment {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
}

pub(crate) fn resolve(lookup: &dyn EnvironmentLookup) -> Result<ResourceEnvironment, ConfigError> {
    Ok(ResourceEnvironment {
        api_base: lookup
            .lookup(API_BASE_VARIABLE)
            .filter(|value| !value.is_empty()),
        // the resource key is the chat key, so both read MISTRAL_API_KEY through one resolution
        api_key: options::resolve(lookup)?.api_key,
    })
}

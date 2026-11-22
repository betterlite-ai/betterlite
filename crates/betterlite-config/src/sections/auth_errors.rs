use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellm {
    #[serde(default)]
    log_auth_failure_key_identity: Option<bool>,
    #[serde(default)]
    log_client_error_tracebacks: Option<bool>,
}

// litellm.log_auth_failure_key_identity and litellm.log_client_error_tracebacks, both off by default
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub log_auth_failure_key_identity: bool,
    pub log_client_error_tracebacks: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            log_auth_failure_key_identity: self
                .litellm_settings
                .log_auth_failure_key_identity
                .unwrap_or_default(),
            log_client_error_tracebacks: self
                .litellm_settings
                .log_client_error_tracebacks
                .unwrap_or_default(),
        })
    }
}

use betterlite_config::env_integer::timeout::{RequestTimeout, TimeoutError};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::types::Config;
use thiserror::Error;

mod tests;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error(transparent)]
    Timeout {
        #[from]
        source: TimeoutError,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoreSettings {
    pub request_timeout_seconds: f64,
    pub request_timeout_explicit: bool,
    pub litellm_request_timeout: Option<f64>,
    pub model_timeout_overrides: Vec<Option<f64>>,
}

impl CoreSettings {
    pub fn load(
        config: &Config,
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, SettingsError> {
        let timeout = RequestTimeout::resolve(environment)?;
        Ok(Self {
            request_timeout_seconds: timeout.seconds,
            request_timeout_explicit: timeout.explicitly_set,
            litellm_request_timeout: config.litellm_settings.request_timeout,
            model_timeout_overrides: config
                .model_list
                .iter()
                .map(|model| model.litellm_params.timeout)
                .collect(),
        })
    }
}

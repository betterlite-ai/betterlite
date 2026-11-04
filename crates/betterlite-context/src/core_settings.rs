use betterlite_config::types::Config;

pub const DEFAULT_TIMEOUT_SECONDS: f64 = 6000.0;

#[derive(Debug, Clone, PartialEq)]
pub struct CoreSettings {
    pub request_timeout_seconds: f64,
    pub request_timeout_explicit: bool,
    pub litellm_request_timeout: Option<f64>,
    pub model_timeout_overrides: Vec<Option<f64>>,
}

impl CoreSettings {
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let request_timeout_seconds = config
            .litellm_settings
            .request_timeout
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS);
        Self {
            request_timeout_seconds,
            request_timeout_explicit: config.litellm_settings.request_timeout.is_some(),
            litellm_request_timeout: config.litellm_settings.request_timeout,
            model_timeout_overrides: config
                .model_list
                .iter()
                .map(|model| model.litellm_params.timeout)
                .collect(),
        }
    }
}

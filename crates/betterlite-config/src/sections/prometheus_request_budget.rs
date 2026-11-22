use std::time::Duration;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const TIMEOUT_VARIABLE: &str = "PROMETHEUS_BUDGET_METRICS_PER_REQUEST_TIMEOUT";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // every value is read from the environment, so this section owns no YAML key
    #[serde(skip)]
    _no_yaml_keys: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub per_request_timeout: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            per_request_timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(text) = lookup.lookup(TIMEOUT_VARIABLE) else {
            return Ok(Settings::default());
        };
        let parsed = betterlite_env::number::<f64>(TIMEOUT_VARIABLE, &text)
            .map_err(|source| ConfigError::Environment { source })?;
        let Some(seconds) = parsed else {
            return Ok(Settings::default());
        };
        if !seconds.is_finite() || seconds <= 0.0_f64 {
            // LiteLLM's line; Debug prints the float default as 5.0, like its %s does
            tracing::debug!(
                "[Non-Blocking] Prometheus: invalid {TIMEOUT_VARIABLE}={text}; using default {:?}s.",
                DEFAULT_TIMEOUT.as_secs_f64()
            );
            return Ok(Settings::default());
        }
        Ok(Settings {
            // Duration saturates near 5.8e11 years, so a larger configured value means no timeout
            per_request_timeout: Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX),
        })
    }
}

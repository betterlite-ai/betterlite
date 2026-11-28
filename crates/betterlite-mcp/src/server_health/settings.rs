use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_env::number;

pub const HEALTH_CHECK_TIMEOUT_VARIABLE: &str = "LITELLM_MCP_HEALTH_CHECK_TIMEOUT";

const DEFAULT_HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealthSettings {
    pub health_check_timeout: Duration,
}

impl Default for HealthSettings {
    fn default() -> Self {
        Self {
            health_check_timeout: DEFAULT_HEALTH_CHECK_TIMEOUT,
        }
    }
}

impl HealthSettings {
    #[must_use]
    pub fn from_lookup<E>(environment: &E) -> Self
    where
        E: EnvironmentLookup,
    {
        let given = environment.lookup(HEALTH_CHECK_TIMEOUT_VARIABLE);
        let Some(health_check_timeout) = given.as_deref().and_then(positive_seconds) else {
            if let Some(text) = given.as_deref().filter(|text| !text.trim().is_empty()) {
                tracing::warn!(
                    variable = HEALTH_CHECK_TIMEOUT_VARIABLE,
                    value = text,
                    "the MCP health check timeout is not a positive number of seconds; using 10 seconds"
                );
            }
            return Self::default();
        };
        Self {
            health_check_timeout,
        }
    }
}

// LiteLLM reads the variable as a float of seconds, so only a positive finite number counts
fn positive_seconds(text: &str) -> Option<Duration> {
    let seconds = number::<f64>(HEALTH_CHECK_TIMEOUT_VARIABLE, text).ok()??;
    if !seconds.is_finite() || seconds <= 0.0_f64 {
        return None;
    }
    Duration::try_from_secs_f64(seconds).ok()
}

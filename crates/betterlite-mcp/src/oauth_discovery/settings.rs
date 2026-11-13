use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;

pub const METADATA_TIMEOUT_VARIABLE: &str = "LITELLM_MCP_METADATA_TIMEOUT";

const DEFAULT_METADATA_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiscoverySettings {
    pub metadata_timeout: Duration,
}

impl Default for DiscoverySettings {
    fn default() -> Self {
        Self {
            metadata_timeout: DEFAULT_METADATA_TIMEOUT,
        }
    }
}

impl DiscoverySettings {
    #[must_use]
    pub fn from_lookup<E>(environment: &E) -> Self
    where
        E: EnvironmentLookup,
    {
        let Some(value) = environment.lookup(METADATA_TIMEOUT_VARIABLE) else {
            return Self::default();
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Self::default();
        }
        let parsed = trimmed.parse::<f64>().ok().and_then(timeout_from_seconds);
        let Some(metadata_timeout) = parsed else {
            tracing::warn!(
                variable = METADATA_TIMEOUT_VARIABLE,
                value = trimmed,
                "the MCP metadata timeout is not a positive number of seconds; using 10 seconds"
            );
            return Self::default();
        };
        Self { metadata_timeout }
    }
}

fn timeout_from_seconds(seconds: f64) -> Option<Duration> {
    if !seconds.is_finite() || seconds <= 0.0_f64 {
        return None;
    }
    Duration::try_from_secs_f64(seconds).ok()
}

use std::num::ParseFloatError;

use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};

mod tests;

#[derive(Debug, Error)]
pub enum TimeoutError {
    #[error("invalid number in REQUEST_TIMEOUT: {source}")]
    Invalid {
        #[source]
        source: ParseFloatError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RequestTimeout {
    pub seconds: f64,
    pub explicitly_set: bool,
}

impl RequestTimeout {
    // LiteLLM's package default: request_timeout is 6000 unless a config value or the REQUEST_TIMEOUT environment replaces it
    pub const DEFAULT_SECONDS: f64 = 6000.0;

    pub fn resolve(environment: &dyn EnvironmentLookup) -> Result<Self, TimeoutError> {
        let value = environment.lookup("REQUEST_TIMEOUT");
        let configured = value.as_deref().filter(|text| !text.is_empty());
        let seconds = configured.map_or(Ok(Self::DEFAULT_SECONDS), |text| {
            text.trim()
                .parse()
                .map_err(|source| TimeoutError::Invalid { source })
        })?;
        Ok(Self {
            seconds,
            explicitly_set: configured.is_some(),
        })
    }

    // both reads run before the choice, so an invalid REQUEST_TIMEOUT still fails a config that sets request_timeout
    pub fn resolve_configured<L>(
        configured: Option<RawNumber>,
        lookup: &L,
    ) -> Result<Self, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured_seconds = resolve_number(
            configured,
            "litellm_settings.request_timeout",
            lookup,
            parse_float,
        )?;
        let environment_text = lookup.lookup("REQUEST_TIMEOUT").unwrap_or_default();
        let environment = betterlite_env::number::<f64>("REQUEST_TIMEOUT", &environment_text)?;
        let (seconds, explicitly_set) = configured_seconds
            .map(|seconds| (seconds, true))
            .or_else(|| environment.map(|seconds| (seconds, true)))
            .unwrap_or((Self::DEFAULT_SECONDS, false));
        Ok(Self {
            seconds,
            explicitly_set,
        })
    }
}

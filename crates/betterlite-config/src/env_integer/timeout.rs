use crate::loader::EnvironmentLookup;
use std::num::ParseFloatError;
use thiserror::Error;

mod tests;

#[derive(Debug, Error)]
pub enum TimeoutError {
    #[error("invalid number in REQUEST_TIMEOUT: {source}")]
    Invalid {
        #[source]
        source: ParseFloatError,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct RequestTimeout {
    pub seconds: f64,
    pub explicitly_set: bool,
}

impl RequestTimeout {
    pub fn resolve(environment: &dyn EnvironmentLookup) -> Result<Self, TimeoutError> {
        let value = environment.lookup("REQUEST_TIMEOUT");
        let configured = value.as_deref().filter(|text| !text.is_empty());
        let seconds = configured.map_or(Ok(6000.0_f64), |text| {
            text.trim()
                .parse()
                .map_err(|source| TimeoutError::Invalid { source })
        })?;
        Ok(Self {
            seconds,
            explicitly_set: configured.is_some(),
        })
    }
}

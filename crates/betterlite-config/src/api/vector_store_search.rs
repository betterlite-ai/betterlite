use thiserror::Error;

use crate::env_integer::timeout::{RequestTimeout, TimeoutError};
use crate::loader::EnvironmentLookup;

#[path = "vector_store_search/tests.rs"]
mod tests;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
}

// REQUEST_TIMEOUT is the SDK's timeout in float seconds, 6000 when the environment leaves it unset
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub request_timeout_seconds: f64,
}

impl Settings {
    pub fn load(environment: &dyn EnvironmentLookup) -> Result<Self, SettingsError> {
        Ok(Self {
            request_timeout_seconds: RequestTimeout::resolve(environment)?.seconds,
        })
    }
}

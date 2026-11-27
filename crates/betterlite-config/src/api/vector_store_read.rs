use thiserror::Error;

use crate::env_integer::timeout::{RequestTimeout, TimeoutError};
use crate::loader::EnvironmentLookup;

#[path = "vector_store_read/tests.rs"]
mod tests;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
}

// REQUEST_TIMEOUT as the read transport's fallback: 6000 seconds unless the environment sets it
pub fn loads_read_timeout(environment: &dyn EnvironmentLookup) -> Result<f64, SettingsError> {
    match RequestTimeout::resolve(environment) {
        Ok(timeout) => Ok(timeout.seconds),
        Err(source) => Err(SettingsError::Timeout(source)),
    }
}

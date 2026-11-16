use thiserror::Error;

use crate::guardrail_registry::error::RegistryError;

// No variant holds a parameter value, since the params carry keys
#[derive(Debug, Error)]
pub enum StartupError {
    #[error("guardrail '{guardrail_name}' failed to start: {source}")]
    Guardrail {
        guardrail_name: String,
        #[source]
        source: RegistryError,
    },
    #[error("{message}")]
    Legacy { message: String },
    #[error("guardrail database sync failed: {source}")]
    Database {
        #[source]
        source: RegistryError,
    },
}

use thiserror::Error;

use crate::callback_config::error::ValidationError;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("no callback adapter is installed for '{name}'")]
    UnavailableAdapter { name: String },
    #[error(transparent)]
    Validation {
        #[from]
        source: ValidationError,
    },
}

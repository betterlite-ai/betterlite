use crate::registry::RegistryError;

#[derive(Debug, thiserror::Error)]
pub enum PolicyParseError {
    #[error(transparent)]
    Shape(#[from] serde_json::Error),
    #[error(transparent)]
    Policy(#[from] RegistryError),
}

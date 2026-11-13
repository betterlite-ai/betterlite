use thiserror::Error;

use crate::private_json::error::PrivateJsonError;

#[derive(Debug, Error)]
pub enum TraceRowError {
    #[error("a nested field could not be written as JSON")]
    NestedField(#[source] PrivateJsonError),
}

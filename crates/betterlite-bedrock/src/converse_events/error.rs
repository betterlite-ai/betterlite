use std::convert::Infallible;

use thiserror::Error;

use crate::converse_usage::error::UsageError;

#[derive(Debug, Error)]
pub enum ConverseEventError {
    #[error("the Bedrock event field {field} has a type the Converse event does not allow")]
    FieldType { field: &'static str },
    #[error("the Bedrock event field {field} is required")]
    Missing { field: &'static str },
    #[error("the Bedrock content block index is outside the signed 64-bit range")]
    IndexRange,
    #[error("the Bedrock tool call index overflowed")]
    IndexOverflow,
    #[error("the Bedrock usage block could not be projected")]
    Usage(#[from] UsageError<Infallible>),
}

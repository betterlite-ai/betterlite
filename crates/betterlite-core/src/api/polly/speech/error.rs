use std::error::Error as StdError;

use betterlite_sigv4::error::SigV4Error;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpeechError {
    #[error(
        "Invalid AWS region format: '{region}'. Region names must contain only lowercase letters, digits, and hyphens."
    )]
    InvalidRegion { region: String },
    #[error("the AWS Polly parameter {field} must be a string")]
    WrongType { field: &'static str },
    #[error("the AWS Polly request cannot be signed")]
    Signing(#[source] SigV4Error),
    #[error("the AWS Polly credentials could not be resolved")]
    Credentials {
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
}

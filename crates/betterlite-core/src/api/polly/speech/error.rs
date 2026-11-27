use aws_sigv4::http_request::SigningError as AwsSigningError;
use aws_sigv4::sign::v4::signing_params::BuildError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpeechError {
    #[error(
        "Invalid AWS region format: '{region}'. Region names must contain only lowercase letters, digits, and hyphens."
    )]
    InvalidRegion { region: String },
    #[error("the AWS Polly parameter {field} must be a string")]
    WrongType { field: &'static str },
    #[error("the AWS Polly signing parameters are invalid")]
    SigningParameters(#[source] Box<BuildError>),
    #[error("the AWS Polly request cannot be signed")]
    Signable(#[source] Box<AwsSigningError>),
    #[error("signing the AWS Polly request failed")]
    Sign(#[source] Box<AwsSigningError>),
    #[error("the AWS Polly credentials could not be resolved")]
    Credentials {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

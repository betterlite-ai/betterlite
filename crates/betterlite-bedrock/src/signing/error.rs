use crate::aws_endpoints::error::AwsEndpointError;
use crate::aws_settings::AuthError;
use crate::credentials::error::CredentialError;
use aws_sigv4::http_request::SigningError as SigV4Error;
use aws_sigv4::sign::v4::signing_params::BuildError;
use tokio::task::JoinError;
#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    // botocore's own message, which callers of get_request_headers can see
    #[error("Unable to locate credentials")]
    NoCredentials,
    #[error("the AWS region is required to sign the request")]
    MissingRegion,
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("the AWS credential lookup failed")]
    Credential(#[source] CredentialError),
    #[error("the signing parameters are invalid")]
    Parameters(#[source] BuildError),
    #[error("the request cannot be signed")]
    Signable(#[source] SigV4Error),
    #[error("the signing task failed")]
    Join(#[source] JoinError),
    #[error("the signing executor is closed")]
    ExecutorClosed,
}

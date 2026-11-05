use aws_credential_types::provider::error::CredentialsError;
use aws_sdk_sts::{error::SdkError, operation::assume_role::AssumeRoleError};
use aws_sigv4::{http_request::SigningError, sign::v4::signing_params::BuildError};
use http::header::{InvalidHeaderValue, ToStrError};

#[derive(Debug, thiserror::Error)]
pub enum McpSigV4Error {
    #[error(
        "No AWS credentials found. Provide aws_access_key_id and aws_secret_access_key, set aws_role_name for STS AssumeRole, or configure default credentials (env vars, ~/.aws/credentials, instance profile)."
    )]
    NoCredentials,
    #[error("STS AssumeRole failed")]
    AssumeRole {
        #[source]
        source: Box<SdkError<AssumeRoleError>>,
    },
    #[error("STS AssumeRole returned no credentials")]
    AssumeRoleWithoutCredentials,
    #[error("the AWS credential provider failed")]
    Provider {
        #[source]
        source: CredentialsError,
    },
    #[error("header {name} cannot be signed because its value is not visible ASCII")]
    HeaderValue {
        name: String,
        #[source]
        source: ToStrError,
    },
    #[error("the Date header is not supported: SigV4 replaces it with a formatted request time")]
    DateHeader,
    #[error("SigV4 signing parameters are invalid")]
    SigningParams {
        #[source]
        source: BuildError,
    },
    #[error("the request cannot be signed")]
    SignableRequest {
        #[source]
        source: SigningError,
    },
    #[error("signing the request failed")]
    Sign {
        #[source]
        source: SigningError,
    },
    #[error("the signature header {name} is not a valid header value")]
    SignatureHeader {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
}

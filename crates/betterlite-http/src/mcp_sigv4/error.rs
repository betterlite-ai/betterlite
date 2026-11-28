use std::time::SystemTimeError;

use aws_credential_types::provider::error::CredentialsError;
use aws_sdk_sts::{error::SdkError, operation::assume_role::AssumeRoleError};
use betterlite_sigv4::error::SigV4Error;

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
    #[error("the clock reads before the Unix epoch, so the role session name has no epoch seconds")]
    ClockBeforeEpoch {
        #[source]
        source: SystemTimeError,
    },
    #[error("the Date header is not supported: SigV4 replaces it with a formatted request time")]
    DateHeader,
    #[error("signing the request failed")]
    Signing {
        #[source]
        source: SigV4Error,
    },
}

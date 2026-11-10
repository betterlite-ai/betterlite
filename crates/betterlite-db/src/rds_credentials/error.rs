use std::time::{SystemTimeError, TryFromFloatSecsError};

use aws_config::profile::ProfileFileLoadError;
use aws_credential_types::provider::error::CredentialsError;
use aws_sdk_sts::error::SdkError;
use aws_sdk_sts::operation::assume_role::AssumeRoleError;
use aws_sdk_sts::operation::assume_role_with_web_identity::AssumeRoleWithWebIdentityError;
use aws_sigv4::http_request::SigningError;
use aws_sigv4::sign::v4;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RdsCredentialsError {
    #[error("AWS region not set: set AWS_REGION_NAME or AWS_REGION env variable or in .env file")]
    MissingRegion,
    #[error("OIDC token could not be retrieved from secret manager.")]
    MissingOidcToken,
    // the failing name is left out because a caller may pass the token itself as the secret name
    #[error("the AWS secret lookup failed")]
    Secret {
        #[source]
        source: SecretLookupError,
    },
    #[error("the AWS secret is a boolean, not a string")]
    SecretNotText,
    #[error("partial AWS credentials: missing {missing}")]
    PartialCredentials { missing: &'static str },
    #[error("AWS credentials are not available")]
    NoCredentials,
    #[error("the AWS credential provider failed")]
    Provider {
        #[source]
        source: CredentialsError,
    },
    #[error("the AWS profile {profile} did not provide credentials")]
    Profile {
        profile: String,
        #[source]
        source: CredentialsError,
    },
    #[error("the AWS profile files could not be read for {profile}")]
    ProfileFile {
        profile: String,
        #[source]
        source: ProfileFileLoadError,
    },
    #[error("the AWS profile {profile} was not found")]
    MissingProfile { profile: String },
    #[error("STS AssumeRoleWithWebIdentity failed")]
    AssumeRoleWithWebIdentity {
        #[source]
        source: Box<SdkError<AssumeRoleWithWebIdentityError>>,
    },
    #[error("STS AssumeRole failed")]
    AssumeRole {
        #[source]
        source: Box<SdkError<AssumeRoleError>>,
    },
    #[error("the STS response carries no credentials")]
    StsWithoutCredentials,
    #[error("the STS response has no {field}")]
    StsMissingField { field: &'static str },
    #[error("the AWS timeout is outside the duration range")]
    Timeout {
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the RDS signing parameters are invalid")]
    SigningParams {
        #[source]
        source: v4::signing_params::BuildError,
    },
    #[error("the RDS authentication request cannot be signed")]
    SignableRequest {
        #[source]
        source: SigningError,
    },
    #[error("signing the RDS authentication token failed")]
    Sign {
        #[source]
        source: SigningError,
    },
    #[error("the signing clock reads before the Unix epoch")]
    ClockBeforeEpoch {
        #[source]
        source: SystemTimeError,
    },
}

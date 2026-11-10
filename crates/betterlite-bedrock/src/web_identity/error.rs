use crate::aws_endpoints::error::AwsEndpointError;
use crate::aws_settings::error::AwsAuthError;
use aws_sdk_sts::error::{BuildError, SdkError};
use aws_sdk_sts::operation::assume_role::AssumeRoleError;
use aws_sdk_sts::operation::assume_role_with_web_identity::AssumeRoleWithWebIdentityError;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use std::io;
use std::path::PathBuf;
use std::string::FromUtf8Error;
#[derive(Debug, thiserror::Error)]
pub enum WebIdentityError {
    #[error("{0}")]
    Auth(Box<AwsAuthError>),
    #[error("{error}")]
    RejectedToken {
        error: Box<AwsAuthError>,
        #[source]
        source: Box<SdkError<AssumeRoleWithWebIdentityError>>,
    },
    #[error(transparent)]
    Secret(#[from] SecretLookupError),
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error("cannot read the web identity token file {}", path.display())]
    TokenFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the web identity token file {} is not UTF-8", path.display())]
    TokenFileEncoding {
        path: PathBuf,
        #[source]
        source: FromUtf8Error,
    },
    #[error("{0}")]
    MissingField(String),
    #[error("the STS web identity call failed")]
    WebIdentity {
        #[source]
        source: Box<SdkError<AssumeRoleWithWebIdentityError>>,
    },
    #[error("the STS assume role call failed")]
    AssumeRole {
        #[source]
        source: Box<SdkError<AssumeRoleError>>,
    },
    #[error("the aws_web_identity_token secret is not text")]
    SecretType,
    #[error("the {field} parameter is not part of the AssumeRoleWithWebIdentity API")]
    UnsupportedParameter { field: &'static str },
    #[error("the policy document does not serialize")]
    Policy {
        #[source]
        source: serde_json::Error,
    },
    #[error("the STS session tag does not build")]
    Tag {
        #[source]
        source: BuildError,
    },
    #[error("the STS credential expiry is outside the representable range")]
    Expiry,
}

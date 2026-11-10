use std::io;
use std::path::PathBuf;
use std::string::FromUtf8Error;

use aws_sdk_sts::error::{BuildError, SdkError};
use aws_sdk_sts::operation::assume_role::AssumeRoleError;
use aws_sdk_sts::operation::assume_role_with_web_identity::AssumeRoleWithWebIdentityError;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::aws_settings::error::{AWS_CONSOLE_URL, AwsAuthError};

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
    #[error("the STS web identity response has no {field}")]
    WebIdentityResponse { field: &'static str },
    #[error("the STS assume role response has no {field}")]
    AssumeRoleResponse { field: &'static str },
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
    #[error("the {field} secret is not text")]
    SecretType { field: &'static str },
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

pub(super) fn auth_error(status: u16, message: String) -> AwsAuthError {
    let request = ErrorRequest {
        method: "POST".to_owned(),
        url: AWS_CONSOLE_URL.to_owned(),
    };
    AwsAuthError {
        status,
        message,
        response: ErrorResponse {
            status_code: status,
            request: request.clone(),
        },
        request,
    }
}

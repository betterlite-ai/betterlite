use std::io;
use std::path::PathBuf;
use std::str::Utf8Error;

use thiserror::Error;

use super::ports::{AzureOidcError, OidcHttpError};
use crate::oidc::error::OidcError;

#[derive(Debug, Error)]
pub enum SecretLookupError {
    #[error("Google OIDC provider failed")]
    GoogleOidcFailed,
    #[error("Github OIDC provider failed")]
    GithubOidcFailed,
    #[error("CIRCLE_OIDC_TOKEN not found in environment")]
    CircleCiTokenMissing,
    #[error("CIRCLE_OIDC_TOKEN_V2 not found in environment")]
    CircleCiTokenV2Missing,
    #[error(
        "ACTIONS_ID_TOKEN_REQUEST_URL or ACTIONS_ID_TOKEN_REQUEST_TOKEN not found in environment"
    )]
    GithubRequestMissing,
    #[error("Azure OIDC provider failed: Azure OIDC provider returned None token")]
    AzureProviderReturnedNone,
    #[error("Azure OIDC provider failed: {source}")]
    AzureProvider {
        #[source]
        source: AzureOidcError,
    },
    #[error("Environment variable {name} not found")]
    EnvironmentVariableMissing { name: String },
    #[error("Unsupported OIDC provider")]
    UnsupportedOidcProvider,
    #[error("OIDC secret name needs a provider and an audience")]
    MissingOidcAudience,
    #[error(transparent)]
    OidcPath(#[from] OidcError),
    #[error("OIDC request failed: {source}")]
    OidcRequest {
        #[source]
        source: OidcHttpError,
    },
    #[error("OIDC token file {} could not be read", path.display())]
    TokenFileRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Google OIDC token response is not UTF-8")]
    GoogleTokenUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("Github OIDC token response is not a token document")]
    GithubTokenNotObject,
    #[error("Github OIDC token response is not a token document")]
    GithubTokenDocument {
        #[source]
        source: serde_json::Error,
    },
}

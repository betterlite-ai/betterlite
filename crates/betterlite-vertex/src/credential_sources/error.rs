use std::io;
use std::path::PathBuf;

use betterlite_config::sections::vertex_credential_sources::credential_source::CredentialDecodeError;
use google_cloud_auth::build_errors::Error as BuildError;
use google_cloud_auth::errors::CredentialsError;

use crate::aws_federation::FederationError;

#[derive(Debug, thiserror::Error)]
pub enum CredentialSourceError {
    #[error(
        "Unable to load vertex credentials from environment. Ensure the JSON is valid (check for unescaped newlines in private_key). Parse error: {source}"
    )]
    Credentials {
        #[source]
        source: CredentialDecodeError,
    },
    #[error("the vertex credentials file {} could not be read", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the vertex credentials could not be decoded")]
    Decode(#[source] CredentialDecodeError),
    #[error("the vertex credentials document is not valid JSON")]
    Document(#[source] serde_json::Error),
    #[error("the identity pool credential_source names an environment_id that is not an AWS one")]
    InvalidIdentityPoolEnvironment,
    #[error("the {kind} Google credential could not be built")]
    Build {
        kind: &'static str,
        #[source]
        source: BuildError,
    },
    #[error("the Google credential could not be refreshed")]
    Refresh(#[source] CredentialsError),
    #[error("Could not resolve project_id")]
    MissingProject,
    #[error("the AWS workload identity federation could not be refreshed")]
    Federation(#[source] FederationError),
}

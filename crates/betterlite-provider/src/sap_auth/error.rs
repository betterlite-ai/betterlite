use std::io;
use std::path::PathBuf;

use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use thiserror::Error;

use super::token::TokenTransportError;

#[derive(Debug, Error)]
pub enum SapAuthError {
    #[error("No credentials found in any source")]
    NoCredentials,
    #[error(
        "Unable to locate profile config file at '{}' in AICORE_HOME '{}'",
        path.display(),
        home.display()
    )]
    ConfigFileNotFound { path: PathBuf, home: PathBuf },
    #[error("{} is not valid JSON. Please fix or remove it!", path.display())]
    ConfigNotJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("the SAP AI Core config file {} could not be read", path.display())]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "SAP AI Core credentials not found. Please provide credentials by setting appropriate environment variables (e.g. AICORE_CLIENT_ID, AICORE_CLIENT_SECRET, etc.)"
    )]
    MissingCredentials,
    #[error(
        "SAP AI Core credentials are incomplete. Invalid credentials: provide exactly one of client_secret, (cert_str & key_str), or (cert_file_path & key_file_path)."
    )]
    IncompleteCredentials,
    #[error("Token request failed: {text}")]
    TokenRequest {
        text: String,
        #[source]
        source: Box<TokenFailure>,
    },
    #[error("no orchestration deployment was found")]
    NoOrchestrationDeployment,
    #[error("the deployment listing holds an entry that is not a JSON object")]
    DeploymentEntry,
    #[error("the deployment configuration {configuration_id} could not be read")]
    ConfigurationLookup {
        configuration_id: String,
        #[source]
        source: ConfigurationLookupError,
    },
    #[error("a SAP AI Core credential holds a character a header cannot carry")]
    InvalidHeader {
        #[source]
        source: InvalidHeaderValue,
    },
}

#[derive(Debug, Error)]
pub enum TokenFailure {
    #[error("the token response is not the JSON the token endpoint documents")]
    Response(#[source] serde_json::Error),
    #[error("the token expiry of {0} seconds is out of range")]
    Expiry(i64),
    #[error("{0}")]
    Transport(#[source] TokenTransportError),
}

#[derive(Debug, Error)]
pub enum ConfigurationLookupError {
    #[error("the deployment configuration {0} was not found")]
    Missing(String),
    #[error("the deployment configuration request failed")]
    Transport(#[source] Box<TransportError>),
    #[error("the deployment configuration response is not JSON")]
    Decode(#[source] serde_json::Error),
}

use std::io;
use std::path::PathBuf;

use http::header::InvalidHeaderValue;
use thiserror::Error;

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
    ConfigNotJson { path: PathBuf },
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
    TokenRequest { text: String },
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
#[error("{message}")]
pub struct ConfigurationLookupError {
    pub message: String,
}

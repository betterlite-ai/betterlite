use thiserror::Error;

use crate::secret_name::UnsafeSecretNameError;

#[derive(Debug, Error)]
pub enum CyberArkError<E> {
    #[error("CyberArk Conjur request failed: {0}")]
    Transport(#[source] E),
    #[error("CyberArk Conjur answered with status {status}")]
    Status { status: u16 },
    #[error("Could not authenticate to CyberArk Conjur: {0}")]
    Authentication(#[source] Box<Self>),
    #[error(transparent)]
    UnsafeName(#[from] UnsafeSecretNameError),
}

#[derive(Debug, Error)]
pub enum CyberArkInitError {
    #[error(
        "Missing CyberArk credentials. Please set CYBERARK_API_KEY or both CYBERARK_CLIENT_CERT and CYBERARK_CLIENT_KEY in your environment."
    )]
    MissingCredentials,
    #[error(transparent)]
    RefreshInterval(#[from] betterlite_env::EnvError),
}

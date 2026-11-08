use thiserror::Error;

use crate::secret_name::UnsafeSecretNameError;
use crate::vault::error::VaultError;

#[derive(Debug, Error)]
pub(crate) enum VaultOperationError<E> {
    #[error("the Vault request did not complete")]
    Transport {
        #[source]
        source: E,
    },
    #[error("Vault answered with status {status}")]
    Status { status: u16 },
    #[error("the Vault response is longer than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Vault response body is not a JSON object")]
    InvalidBody,
    #[error("the Vault request timed out")]
    Timeout,
    #[error("could not authenticate to Vault")]
    Authentication {
        #[source]
        source: VaultError<E>,
    },
    #[error("the secret name is not safe for a Vault path")]
    Target {
        #[source]
        source: UnsafeSecretNameError,
    },
    #[error("could not encode the Vault request body")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

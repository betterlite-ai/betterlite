use std::error::Error as StdError;

use betterlite_db::model_secrets::ModelSecretsError;
use thiserror::Error;
use tokio::task::JoinError;

use super::environment::EnvironmentError;
use crate::credentials::error::CredentialError;
use crate::db_row_decrypt::error::DbRowDecryptError;
use crate::env_value_codec::EnvValueCodecError;

// Store and Sink take no From impl: either could itself be one of the concrete errors, and the impls would overlap.
#[derive(Debug, Error)]
pub enum RotationError<Store, Sink>
where
    Store: StdError + Send + 'static,
    Sink: StdError + Send + 'static,
{
    #[error(transparent)]
    Decrypt(#[from] DbRowDecryptError),
    #[error("the model decryption task failed: {0}")]
    DecryptTask(#[from] JoinError),
    #[error("the model decryption task never ran")]
    DecryptTaskMissing,
    #[error(transparent)]
    Model(#[from] ModelSecretsError),
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    #[error("the decrypted environment variables could not be encrypted")]
    Encrypt(#[from] EnvValueCodecError),
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("the rotation store failed")]
    Store(#[source] Store),
    #[error("the model-table invalidation failed")]
    Invalidation(#[source] Sink),
}

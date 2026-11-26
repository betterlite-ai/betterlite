use std::error::Error;

// The source raises RuntimeError for both preconditions; the route owner maps these variants onto its 500 envelope.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError<Store, Rotation>
where
    Store: Error + Send + 'static,
    Rotation: Error + Send + 'static,
{
    #[error(
        "general_settings.encryption_algorithm must be aes-256-gcm to migrate stored ciphertext"
    )]
    AlgorithmGate,
    #[error(
        "no LITELLM_SALT_KEY or general_settings.master_key is set, so the covered stores cannot be re-encrypted"
    )]
    MissingKey,
    #[error("the stored migration rows could not be read or written")]
    Store(#[source] Store),
    #[error("the covered stores could not be re-encrypted")]
    Rotation(#[source] Rotation),
}

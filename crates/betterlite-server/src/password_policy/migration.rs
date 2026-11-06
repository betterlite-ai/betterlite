use std::error::Error;
use std::future::Future;

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use super::hashing::{HashError, PasswordHasher, SCRYPT_PREFIX, SaltSource, is_sha256_hex};

#[derive(Debug)]
pub struct StoredPassword {
    pub user_id: String,
    pub password: SecretString,
}

pub trait PasswordStore: Sync {
    type Error: Error + 'static;

    fn users_with_passwords(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredPassword>, Self::Error>> + Send;

    fn set_password(
        &self,
        user_id: &str,
        stored_hash: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

#[derive(Debug, Error)]
pub enum MigrationError<E> {
    #[error("the user store failed")]
    Store(#[source] E),
    #[error(transparent)]
    Hash(#[from] HashError),
}

// SHA256 hashes stay as they are and move on the next login; only plaintext rows are rewritten, one at a time.
pub async fn migrate_passwords_to_scrypt<P, S>(
    store: &P,
    hasher: &PasswordHasher<S>,
) -> Result<String, MigrationError<P::Error>>
where
    P: PasswordStore,
    S: SaltSource,
{
    let users = store
        .users_with_passwords()
        .await
        .map_err(MigrationError::Store)?;
    let plaintext: Vec<&StoredPassword> = users
        .iter()
        .filter(|user| {
            let stored = user.password.expose_secret();
            !stored.is_empty() && !stored.starts_with(SCRYPT_PREFIX) && !is_sha256_hex(stored)
        })
        .collect();
    if plaintext.is_empty() {
        return Ok("No plaintext passwords found".to_owned());
    }
    for user in &plaintext {
        let stored_hash = hasher.hash(&user.password).await?;
        store
            .set_password(&user.user_id, &stored_hash)
            .await
            .map_err(MigrationError::Store)?;
    }
    Ok(format!(
        "Migrated {} plaintext passwords to scrypt",
        plaintext.len()
    ))
}

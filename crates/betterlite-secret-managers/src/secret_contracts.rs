use std::error::Error;
use std::future::Future;
use std::pin::Pin;

use secrecy::SecretString;

pub mod options;
mod tests;

use options::{DeleteOptions, SecretOptions, SecretRotation, SecretWrite};

pub type SecretFuture<'borrowed, T, E> =
    Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'borrowed>>;

#[derive(Debug, thiserror::Error)]
pub enum RotationError<E> {
    #[error("current secret was not found")]
    CurrentSecretNotFound,
    #[error("new secret could not be verified")]
    NewSecretNotVerified,
    #[error("rotation requires distinct secret names")]
    SameName,
    #[error("secret operation failed")]
    Operation {
        #[source]
        source: E,
    },
}

pub trait BaseSecretManager: Sync {
    type Params: Sync;
    type Response: Send;
    type Error: Error + Send + Sync + 'static;

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: SecretOptions<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Option<SecretString>, Self::Error>;

    fn sync_read_secret(
        &self,
        secret_name: &str,
        options: SecretOptions<'_, Self::Params>,
    ) -> Result<Option<SecretString>, Self::Error>;

    fn async_write_secret<'borrowed>(
        &'borrowed self,
        write: SecretWrite<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, Self::Error>;

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: DeleteOptions<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, Self::Error>;

    fn async_rotate_secret<'borrowed>(
        &'borrowed self,
        rotation: SecretRotation<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, RotationError<Self::Error>> {
        Box::pin(async move {
            if rotation.current_secret_name == rotation.new_secret_name {
                return Err(RotationError::SameName);
            }
            self.async_read_secret(rotation.current_secret_name, rotation.options)
                .await
                .map_err(|source| RotationError::Operation { source })?
                .ok_or(RotationError::CurrentSecretNotFound)?;
            let description = format!("Rotated from {}", rotation.current_secret_name);
            let response = self
                .async_write_secret(SecretWrite {
                    secret_name: rotation.new_secret_name,
                    secret_value: rotation.new_secret_value,
                    description: Some(&description),
                    options: rotation.options,
                    tags: None,
                })
                .await
                .map_err(|source| RotationError::Operation { source })?;
            self.async_read_secret(rotation.new_secret_name, rotation.options)
                .await
                .map_err(|source| RotationError::Operation { source })?
                .ok_or(RotationError::NewSecretNotVerified)?;
            self.async_delete_secret(
                rotation.current_secret_name,
                DeleteOptions {
                    recovery_window_in_days: 7,
                    operation: rotation.options,
                },
            )
            .await
            .map_err(|source| RotationError::Operation { source })?;
            Ok(response)
        })
    }
}

use secrecy::SecretString;
use uuid::Uuid;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{AwsOptionalParams, AwsTransport, CredentialResolver};
use crate::aws_secret_reads::AwsReadError;
use crate::secret_contracts::RotationError;
use crate::secret_contracts::options::{SecretRotation, SecretTimeout};

use super::SecretActionResponse;
use super::put_value::async_put_secret_value;
use rotate_write::write_rotated;

mod rotate_write;

const ROTATION_DELETE_DAYS: u32 = 7;

pub(super) async fn rotate<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    rotation: SecretRotation<'_, AwsOptionalParams>,
    next_token: fn() -> Uuid,
) -> Result<SecretActionResponse, RotationError<AwsReadError<Resolver::Error, Transport::Error>>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    if rotation.current_secret_name == rotation.new_secret_name {
        return async_put_secret_value(
            manager,
            rotation.current_secret_name,
            rotation.new_secret_value,
            rotation.options,
            next_token,
        )
        .await
        .map_err(|source| RotationError::Operation { source });
    }
    rotate_to_new_name(manager, &rotation, next_token).await
}

// the steps BaseSecretManager's default rotation takes, which Rust cannot call once this type overrides it
async fn rotate_to_new_name<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    rotation: &SecretRotation<'_, AwsOptionalParams>,
    next_token: fn() -> Uuid,
) -> Result<SecretActionResponse, RotationError<AwsReadError<Resolver::Error, Transport::Error>>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    let mut optional_params = rotation.options.optional_params.map(super::copy_params);
    let timeout = rotation.options.timeout;
    let current = read_rotated(
        manager,
        rotation.current_secret_name,
        optional_params.as_mut(),
        timeout,
    )
    .await?;
    if current.is_none() {
        return Err(RotationError::CurrentSecretNotFound);
    }
    let description = format!("Rotated from {}", rotation.current_secret_name);
    let written = write_rotated(manager, rotation, &description, next_token).await?;
    let created = read_rotated(
        manager,
        rotation.new_secret_name,
        optional_params.as_mut(),
        timeout,
    )
    .await?;
    if created.is_none() {
        return Err(RotationError::NewSecretNotVerified);
    }
    super::delete::delete(
        manager,
        rotation.current_secret_name,
        ROTATION_DELETE_DAYS,
        optional_params.as_mut(),
        timeout,
    )
    .await
    .map_err(|source| RotationError::Operation { source })?;
    Ok(written)
}

async fn read_rotated<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    secret_name: &str,
    optional_params: Option<&mut AwsOptionalParams>,
    timeout: Option<SecretTimeout>,
) -> Result<Option<SecretString>, RotationError<AwsReadError<Resolver::Error, Transport::Error>>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    manager
        .async_read_secret(secret_name, optional_params, timeout, None)
        .await
        .map_err(|source| RotationError::Operation { source })
}

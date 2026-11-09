use uuid::Uuid;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{AwsOptionalParams, AwsTransport, CredentialResolver};
use crate::aws_secret_reads::AwsReadError;
use crate::secret_contracts::RotationError;
use crate::secret_contracts::options::{SecretRotation, SecretWrite};

use super::super::SecretActionResponse;

pub(super) async fn write_rotated<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    rotation: &SecretRotation<'_, AwsOptionalParams>,
    description: &str,
    next_token: fn() -> Uuid,
) -> Result<SecretActionResponse, RotationError<AwsReadError<Resolver::Error, Transport::Error>>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    super::super::write::write(
        manager,
        SecretWrite {
            secret_name: rotation.new_secret_name,
            secret_value: rotation.new_secret_value,
            description: Some(description),
            options: rotation.options,
            tags: None,
        },
        next_token,
    )
    .await
    .map_err(|source| RotationError::Operation { source })
}

use std::time::Duration;

use secrecy::ExposeSecret as _;

use crate::cyberark::operation::{OperationStatus, SecretOperation};
use crate::secret_contracts::options::{DeleteOptions, SecretRotation, SecretWrite};
use crate::vault::HashicorpSecretManager;
use crate::vault::target::{VaultSecretParams, VaultSecretTarget};
use crate::vault::transport::{
    MAX_RESPONSE_BYTES, VaultMethod, VaultRequest, VaultResponse, VaultTransport,
};

use super::error::VaultOperationError;
use super::result::VaultOperationResult;
use super::{delete, write};

enum FetchProblem {
    Timeout,
    Status(VaultOperationResult),
}

pub(super) async fn rotate<T>(
    manager: &HashicorpSecretManager<T>,
    rotation: SecretRotation<'_, VaultSecretParams>,
) -> VaultOperationResult
where
    T: VaultTransport,
{
    match outcome(manager, &rotation).await {
        Ok(result) => result,
        Err(VaultOperationError::Timeout) => {
            tracing::error!("timeout during secret rotation in Hashicorp Vault");
            VaultOperationResult::Status(SecretOperation {
                status: OperationStatus::Error,
                message: "Timeout error occurred".to_owned(),
            })
        }
        Err(error) => {
            tracing::error!("error rotating secret in Hashicorp Vault: {error}");
            VaultOperationResult::Status(SecretOperation {
                status: OperationStatus::Error,
                message: error.to_string(),
            })
        }
    }
}

async fn outcome<T>(
    manager: &HashicorpSecretManager<T>,
    rotation: &SecretRotation<'_, VaultSecretParams>,
) -> Result<VaultOperationResult, VaultOperationError<T::Error>>
where
    T: VaultTransport,
{
    let timeout = super::resolve_timeout(rotation.options.timeout);
    let current_target = manager
        .build_secret_target(
            rotation.current_secret_name,
            rotation.options.optional_params,
        )
        .map_err(|source| VaultOperationError::Target { source })?;
    if let Some(result) = check_current(manager, rotation, &current_target, timeout).await? {
        return Ok(result);
    }
    let description = format!("Rotated from {}", rotation.current_secret_name);
    let create = write::write(
        manager,
        SecretWrite {
            secret_name: rotation.new_secret_name,
            secret_value: rotation.new_secret_value,
            description: Some(&description),
            options: rotation.options,
            tags: None,
        },
    )
    .await;
    if matches!(&create, VaultOperationResult::Status(status) if status.status == OperationStatus::Error)
    {
        return Ok(create);
    }
    let new_target = manager
        .build_secret_target(rotation.new_secret_name, rotation.options.optional_params)
        .map_err(|source| VaultOperationError::Target { source })?;
    if let Some(result) = verify(manager, rotation, &new_target, timeout).await? {
        return Ok(result);
    }
    finish(manager, rotation, &current_target, &new_target).await;
    Ok(create)
}

async fn check_current<T>(
    manager: &HashicorpSecretManager<T>,
    rotation: &SecretRotation<'_, VaultSecretParams>,
    target: &VaultSecretTarget,
    timeout: Duration,
) -> Result<Option<VaultOperationResult>, VaultOperationError<T::Error>>
where
    T: VaultTransport,
{
    let missing = format!("Current secret {} not found", rotation.current_secret_name);
    match fetch_staged(
        manager,
        target,
        timeout,
        missing,
        "Error checking current secret",
    )
    .await
    {
        Ok(_response) => Ok(None),
        Err(FetchProblem::Timeout) => Err(VaultOperationError::Timeout),
        Err(FetchProblem::Status(result)) => Ok(Some(result)),
    }
}

async fn verify<T>(
    manager: &HashicorpSecretManager<T>,
    rotation: &SecretRotation<'_, VaultSecretParams>,
    target: &VaultSecretTarget,
    timeout: Duration,
) -> Result<Option<VaultOperationResult>, VaultOperationError<T::Error>>
where
    T: VaultTransport,
{
    let missing = format!("Failed to verify new secret {}", rotation.new_secret_name);
    let response = match fetch_staged(
        manager,
        target,
        timeout,
        missing,
        "Error verifying new secret",
    )
    .await
    {
        Ok(response) => response,
        Err(FetchProblem::Timeout) => return Err(VaultOperationError::Timeout),
        Err(FetchProblem::Status(result)) => return Ok(Some(result)),
    };
    let parsed =
        match super::response::vault_secret_read_body(response.body.expose_secret().as_bytes()) {
            Ok(parsed) => parsed,
            Err(_source) => {
                tracing::error!("error verifying new secret in Hashicorp Vault");
                return Ok(Some(VaultOperationResult::Status(SecretOperation {
                    status: OperationStatus::Error,
                    message: "Error verifying new secret".to_owned(),
                })));
            }
        };
    if parsed.value(&target.data_key) != Some(rotation.new_secret_value.expose_secret()) {
        tracing::error!("new secret value mismatch in Hashicorp Vault");
        return Ok(Some(VaultOperationResult::Status(SecretOperation {
            status: OperationStatus::Error,
            message: "New secret value mismatch".to_owned(),
        })));
    }
    Ok(None)
}

async fn finish<T>(
    manager: &HashicorpSecretManager<T>,
    rotation: &SecretRotation<'_, VaultSecretParams>,
    current_target: &VaultSecretTarget,
    new_target: &VaultSecretTarget,
) where
    T: VaultTransport,
{
    if rotation.current_secret_name != rotation.new_secret_name {
        let deleted = delete::delete(
            manager,
            rotation.current_secret_name,
            DeleteOptions {
                recovery_window_in_days: 7,
                operation: rotation.options,
            },
        )
        .await;
        if matches!(&deleted, VaultOperationResult::Status(status) if status.status == OperationStatus::Error)
        {
            tracing::warn!("failed to delete old secret after rotation in Hashicorp Vault");
        } else {
            manager.body_cache().delete(&current_target.url);
        }
    }
    manager.body_cache().delete(&new_target.url);
}

async fn fetch_staged<T>(
    manager: &HashicorpSecretManager<T>,
    target: &VaultSecretTarget,
    timeout: Duration,
    missing: String,
    failed: &'static str,
) -> Result<VaultResponse, FetchProblem>
where
    T: VaultTransport,
{
    match fetch(manager, &target.url, timeout).await {
        Ok(response) => Ok(response),
        Err(VaultOperationError::Timeout) => Err(FetchProblem::Timeout),
        Err(VaultOperationError::Status { status: 404 }) => {
            tracing::error!("{missing}");
            Err(FetchProblem::Status(VaultOperationResult::Status(
                SecretOperation {
                    status: OperationStatus::Error,
                    message: missing,
                },
            )))
        }
        Err(error) => {
            tracing::error!("{failed}: {error}");
            Err(FetchProblem::Status(VaultOperationResult::Status(
                SecretOperation {
                    status: OperationStatus::Error,
                    message: format!("{failed}: {error}"),
                },
            )))
        }
    }
}

async fn fetch<T>(
    manager: &HashicorpSecretManager<T>,
    url: &str,
    timeout: Duration,
) -> Result<VaultResponse, VaultOperationError<T::Error>>
where
    T: VaultTransport,
{
    let token = manager
        .request_token()
        .await
        .map_err(|source| VaultOperationError::Authentication { source })?;
    let request = VaultRequest {
        method: VaultMethod::Get,
        url: url.to_owned(),
        token: Some(token),
        namespace: None,
        body: None,
        client_certificate: None,
        timeout,
        follow_redirects: true,
        max_response_bytes: MAX_RESPONSE_BYTES,
    };
    super::send(manager, request).await
}

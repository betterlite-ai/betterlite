use crate::cyberark::operation::{OperationStatus, SecretOperation};
use crate::secret_contracts::options::DeleteOptions;
use crate::vault::HashicorpSecretManager;
use crate::vault::target::VaultSecretParams;
use crate::vault::transport::{MAX_RESPONSE_BYTES, VaultMethod, VaultRequest, VaultTransport};

use super::error::VaultOperationError;
use super::result::VaultOperationResult;

pub(super) async fn delete<T>(
    manager: &HashicorpSecretManager<T>,
    secret_name: &str,
    options: DeleteOptions<'_, VaultSecretParams>,
) -> VaultOperationResult
where
    T: VaultTransport,
{
    match outcome(manager, secret_name, options).await {
        Ok(result) => result,
        Err(error) => {
            tracing::error!("error deleting secret from Hashicorp Vault: {error}");
            VaultOperationResult::Status(SecretOperation {
                status: OperationStatus::Error,
                message: error.to_string(),
            })
        }
    }
}

async fn outcome<T>(
    manager: &HashicorpSecretManager<T>,
    secret_name: &str,
    options: DeleteOptions<'_, VaultSecretParams>,
) -> Result<VaultOperationResult, VaultOperationError<T::Error>>
where
    T: VaultTransport,
{
    let target = manager
        .build_secret_target(secret_name, options.operation.optional_params)
        .map_err(|source| VaultOperationError::Target { source })?;
    let token = manager
        .request_token()
        .await
        .map_err(|source| VaultOperationError::Authentication { source })?;
    let request = VaultRequest {
        method: VaultMethod::Delete,
        url: target.url.clone(),
        token: Some(token),
        namespace: None,
        body: None,
        client_certificate: None,
        timeout: super::resolve_timeout(options.operation.timeout),
        follow_redirects: true,
        max_response_bytes: MAX_RESPONSE_BYTES,
    };
    super::send(manager, request).await?;
    manager.body_cache().delete(&target.url);
    Ok(VaultOperationResult::Status(SecretOperation {
        status: OperationStatus::Success,
        message: format!("Secret {} deleted successfully", target.secret_name),
    }))
}

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::cyberark::operation::{OperationStatus, SecretOperation};
use crate::secret_contracts::options::SecretWrite;
use crate::vault::HashicorpSecretManager;
use crate::vault::response::json_object_body;
use crate::vault::target::VaultSecretParams;
use crate::vault::transport::{MAX_RESPONSE_BYTES, VaultMethod, VaultRequest, VaultTransport};

use super::error::VaultOperationError;
use super::result::{VaultJsonResponse, VaultOperationResult};

#[derive(Serialize)]
struct VaultWriteBody<'borrowed> {
    data: BTreeMap<&'borrowed str, &'borrowed str>,
}

pub(super) async fn write<T>(
    manager: &HashicorpSecretManager<T>,
    write: SecretWrite<'_, VaultSecretParams>,
) -> VaultOperationResult
where
    T: VaultTransport,
{
    match outcome(manager, &write).await {
        Ok(result) => result,
        Err(error) => {
            tracing::error!("error writing secret to Hashicorp Vault: {error}");
            VaultOperationResult::Status(SecretOperation {
                status: OperationStatus::Error,
                message: error.to_string(),
            })
        }
    }
}

async fn outcome<T>(
    manager: &HashicorpSecretManager<T>,
    write: &SecretWrite<'_, VaultSecretParams>,
) -> Result<VaultOperationResult, VaultOperationError<T::Error>>
where
    T: VaultTransport,
{
    let target = manager
        .build_secret_target(write.secret_name, write.options.optional_params)
        .map_err(|source| VaultOperationError::Target { source })?;
    let mut data = BTreeMap::new();
    data.insert(target.data_key.as_str(), write.secret_value.expose_secret());
    if let Some(description) = write.description {
        data.insert("description", description);
    }
    let body = serde_json::to_string(&VaultWriteBody { data })
        .map_err(|source| VaultOperationError::Encode { source })?;
    let token = manager
        .request_token()
        .await
        .map_err(|source| VaultOperationError::Authentication { source })?;
    let request = VaultRequest {
        method: VaultMethod::Post,
        url: target.url,
        token: Some(token),
        namespace: None,
        body: Some(SecretString::from(body)),
        client_certificate: None,
        timeout: super::resolve_timeout(write.options.timeout),
        follow_redirects: true,
        max_response_bytes: MAX_RESPONSE_BYTES,
    };
    let response = super::send(manager, request).await?;
    json_object_body(response.body.expose_secret().as_bytes())
        .map_err(|_source| VaultOperationError::InvalidBody)?;
    Ok(VaultOperationResult::Response(VaultJsonResponse {
        body: response.body,
    }))
}

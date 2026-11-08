use std::convert::Infallible;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::timeout;

use crate::secret_contracts::options::{
    DeleteOptions, SecretOptions, SecretRotation, SecretTimeout, SecretWrite,
};
use crate::secret_contracts::{BaseSecretManager, RotationError, SecretFuture};
use crate::vault::HashicorpSecretManager;
use crate::vault::target::VaultSecretParams;
use crate::vault::transport::{
    MAX_RESPONSE_BYTES, VaultMethod, VaultRequest, VaultResponse, VaultTransport,
};
use error::VaultOperationError;
use result::VaultOperationResult;

mod delete;
mod error;
mod read;
pub mod response;
pub mod result;
mod rotate;
mod tests;
mod write;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

impl<T: VaultTransport> BaseSecretManager for HashicorpSecretManager<T> {
    type Params = VaultSecretParams;
    type Response = VaultOperationResult;
    type Error = Infallible;

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: SecretOptions<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Option<SecretString>, Self::Error> {
        Box::pin(async move { Ok(read::async_read(self, secret_name, options).await) })
    }

    fn sync_read_secret(
        &self,
        secret_name: &str,
        options: SecretOptions<'_, Self::Params>,
    ) -> Result<Option<SecretString>, Self::Error> {
        Ok(read::sync_read(self, secret_name, options))
    }

    fn async_write_secret<'borrowed>(
        &'borrowed self,
        write: SecretWrite<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, Self::Error> {
        Box::pin(async move { Ok(write::write(self, write).await) })
    }

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: DeleteOptions<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, Self::Error> {
        Box::pin(async move { Ok(delete::delete(self, secret_name, options).await) })
    }

    fn async_rotate_secret<'borrowed>(
        &'borrowed self,
        rotation: SecretRotation<'borrowed, Self::Params>,
    ) -> SecretFuture<'borrowed, Self::Response, RotationError<Self::Error>> {
        Box::pin(async move { Ok(rotate::rotate(self, rotation).await) })
    }
}

fn resolve_timeout(timeout: Option<SecretTimeout>) -> Duration {
    match timeout {
        Some(SecretTimeout::Total(limit)) => limit,
        Some(SecretTimeout::Phases {
            connect,
            read,
            write,
            pool,
        }) => [connect, read, write, pool]
            .into_iter()
            .try_fold(Duration::ZERO, |total, phase| {
                phase.map(|limit| total.saturating_add(limit))
            })
            .unwrap_or(DEFAULT_TIMEOUT),
        None => DEFAULT_TIMEOUT,
    }
}

async fn send<T>(
    manager: &HashicorpSecretManager<T>,
    request: VaultRequest,
) -> Result<VaultResponse, VaultOperationError<T::Error>>
where
    T: VaultTransport,
{
    let limit = request.timeout;
    let reply = timeout(limit, manager.transport.send(request))
        .await
        .map_err(|_elapsed| VaultOperationError::Timeout)?;
    checked_reply::<T>(reply)
}

async fn get<T>(
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
    send(manager, request).await
}

fn checked_reply<T>(
    reply: Result<VaultResponse, T::Error>,
) -> Result<VaultResponse, VaultOperationError<T::Error>>
where
    T: VaultTransport,
{
    let response = reply.map_err(|source| VaultOperationError::Transport { source })?;
    if response.body.expose_secret().len() > MAX_RESPONSE_BYTES {
        return Err(VaultOperationError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    if !(200..300).contains(&response.status) {
        return Err(VaultOperationError::Status {
            status: response.status,
        });
    }
    Ok(response)
}

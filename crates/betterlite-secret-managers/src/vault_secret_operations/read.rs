use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use crate::secret_contracts::options::SecretOptions;
use crate::vault::HashicorpSecretManager;
use crate::vault::response::{json_object_body, secret_value};
use crate::vault::target::{VaultSecretParams, VaultSecretTarget};
use crate::vault::transport::{
    MAX_RESPONSE_BYTES, VaultMethod, VaultRequest, VaultResponse, VaultTransport,
};

use super::error::VaultOperationError;

// LiteLLM builds the read client without a timeout parameter, so reads keep the client's 600 second default
const READ_TIMEOUT: Duration = Duration::from_secs(600);

enum CachedLookup {
    Hit(Option<SecretString>),
    Miss,
}

pub(super) async fn async_read<T>(
    manager: &HashicorpSecretManager<T>,
    secret_name: &str,
    options: SecretOptions<'_, VaultSecretParams>,
) -> Option<SecretString>
where
    T: VaultTransport,
{
    let target = target_for(manager, secret_name, options.optional_params)?;
    if let CachedLookup::Hit(value) = cached_lookup(manager, &target) {
        return value;
    }
    let reply = get(manager, &target.url, READ_TIMEOUT).await;
    finish(manager, &target, reply)
}

pub(super) fn sync_read<T>(
    manager: &HashicorpSecretManager<T>,
    secret_name: &str,
    options: SecretOptions<'_, VaultSecretParams>,
) -> Option<SecretString>
where
    T: VaultTransport,
{
    let target = match manager.build_secret_target(secret_name, options.optional_params) {
        Ok(target) => target,
        Err(error) => {
            tracing::error!("error reading secret from Hashicorp Vault: {error}");
            return None;
        }
    };
    if let Some(cached) = manager.body_cache().get(&target.url) {
        return secret_value(cached.expose_secret().as_bytes(), &target.data_key);
    }
    let token = match manager.request_token_blocking() {
        Ok(token) => token,
        Err(error) => {
            tracing::error!("error reading secret from Hashicorp Vault: {error}");
            return None;
        }
    };
    let request = VaultRequest {
        method: VaultMethod::Get,
        url: target.url.clone(),
        token: Some(token),
        namespace: None,
        body: None,
        client_certificate: None,
        timeout: READ_TIMEOUT,
        follow_redirects: true,
        max_response_bytes: MAX_RESPONSE_BYTES,
    };
    let reply = super::checked_reply::<T>(manager.transport.send_blocking(request));
    finish(manager, &target, reply)
}

fn target_for<T>(
    manager: &HashicorpSecretManager<T>,
    secret_name: &str,
    params: Option<&VaultSecretParams>,
) -> Option<VaultSecretTarget>
where
    T: VaultTransport,
{
    match manager.build_secret_target(secret_name, params) {
        Ok(target) => Some(target),
        Err(error) => {
            tracing::error!("error reading secret from Hashicorp Vault: {error}");
            None
        }
    }
}

fn cached_lookup<T>(manager: &HashicorpSecretManager<T>, target: &VaultSecretTarget) -> CachedLookup
where
    T: VaultTransport,
{
    manager.body_cache().get(&target.url).map_or_else(
        || CachedLookup::Miss,
        |cached| {
            CachedLookup::Hit(secret_value(
                cached.expose_secret().as_bytes(),
                &target.data_key,
            ))
        },
    )
}

pub(super) async fn get<T>(
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

fn finish<T>(
    manager: &HashicorpSecretManager<T>,
    target: &VaultSecretTarget,
    reply: Result<VaultResponse, VaultOperationError<T::Error>>,
) -> Option<SecretString>
where
    T: VaultTransport,
{
    let response = match reply {
        Ok(response) => response,
        Err(error) => {
            tracing::error!("error reading secret from Hashicorp Vault: {error}");
            return None;
        }
    };
    if let Err(source) = json_object_body(response.body.expose_secret().as_bytes()) {
        let error = VaultOperationError::<T::Error>::InvalidBody { source };
        tracing::error!("error reading secret from Hashicorp Vault: {error}");
        return None;
    }
    manager.body_cache().set(&target.url, response.body.clone());
    secret_value(response.body.expose_secret().as_bytes(), &target.data_key)
}

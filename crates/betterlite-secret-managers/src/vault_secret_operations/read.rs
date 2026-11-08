use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use crate::secret_contracts::options::SecretOptions;
use crate::vault::HashicorpSecretManager;
use crate::vault::error::VaultError;
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
    let token = token_of(manager.request_token().await)?;
    let request = read_request(target.url.clone(), token);
    finish(manager, &target, super::send(manager, request).await)
}

pub(super) fn sync_read<T>(
    manager: &HashicorpSecretManager<T>,
    secret_name: &str,
    options: SecretOptions<'_, VaultSecretParams>,
) -> Option<SecretString>
where
    T: VaultTransport,
{
    let target = target_for(manager, secret_name, options.optional_params)?;
    match cached_lookup(manager, &target) {
        CachedLookup::Hit(value) => value,
        CachedLookup::Miss => {
            let token = token_of(manager.request_token_blocking())?;
            let request = read_request(target.url.clone(), token);
            let reply = super::checked_reply::<T>(manager.transport.send_blocking(request));
            finish(manager, &target, reply)
        }
    }
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

fn token_of<E>(outcome: Result<SecretString, VaultError<E>>) -> Option<SecretString> {
    match outcome {
        Ok(token) => Some(token),
        Err(error) => {
            tracing::error!("error reading secret from Hashicorp Vault: {error}");
            None
        }
    }
}

fn finish<T>(
    manager: &HashicorpSecretManager<T>,
    target: &VaultSecretTarget,
    reply: Result<VaultResponse, VaultOperationError<T::Error>>,
) -> Option<SecretString>
where
    T: VaultTransport,
{
    match reply {
        Ok(response) => cached_value(manager, &target.url, &target.data_key, &response),
        Err(error) => {
            tracing::error!("error reading secret from Hashicorp Vault: {error}");
            None
        }
    }
}

const fn read_request(url: String, token: SecretString) -> VaultRequest {
    VaultRequest {
        method: VaultMethod::Get,
        url,
        token: Some(token),
        namespace: None,
        body: None,
        client_certificate: None,
        timeout: READ_TIMEOUT,
        follow_redirects: true,
        max_response_bytes: MAX_RESPONSE_BYTES,
    }
}

fn cached_value<T>(
    manager: &HashicorpSecretManager<T>,
    url: &str,
    data_key: &str,
    response: &VaultResponse,
) -> Option<SecretString>
where
    T: VaultTransport,
{
    if json_object_body(response.body.expose_secret().as_bytes()).is_err() {
        tracing::error!(
            "error reading secret from Hashicorp Vault: the response body is not a JSON object"
        );
        return None;
    }
    manager.body_cache().set(url, response.body.clone());
    secret_value(response.body.expose_secret().as_bytes(), data_key)
}

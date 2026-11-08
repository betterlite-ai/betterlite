use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use crate::secret_contracts::options::SecretOptions;
use crate::vault::HashicorpSecretManager;
use crate::vault::response::{json_object_body, secret_value};
use crate::vault::target::VaultSecretParams;
use crate::vault::transport::{
    MAX_RESPONSE_BYTES, VaultMethod, VaultRequest, VaultResponse, VaultTransport,
};

// LiteLLM builds the read client without a timeout parameter, so reads keep the client's 600 second default
const READ_TIMEOUT: Duration = Duration::from_secs(600);

pub(super) async fn async_read<T>(
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
    let token = match manager.request_token().await {
        Ok(token) => token,
        Err(error) => {
            tracing::error!("error reading secret from Hashicorp Vault: {error}");
            return None;
        }
    };
    match super::send(manager, read_request(target.url.clone(), token)).await {
        Ok(response) => cached_value(manager, &target.url, &target.data_key, &response),
        Err(error) => {
            tracing::error!("error reading secret from Hashicorp Vault: {error}");
            None
        }
    }
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
    let reply = manager
        .transport
        .send_blocking(read_request(target.url.clone(), token));
    match super::checked_reply::<T>(reply) {
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

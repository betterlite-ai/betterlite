use std::sync::PoisonError;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use super::error::{LoginMethod, VaultError};
use super::response::login::{VaultLogin, decode_login_response};
use super::settings::VaultAuth;
use super::transport::{
    MAX_RESPONSE_BYTES, VaultMethod, VaultRequest, VaultResponse, VaultTransport,
};
use super::{CachedToken, HashicorpSecretManager};
use crate::cyberark::transport::ClientCertificate;

mod tests;

pub(super) enum PendingLogin {
    Ready(SecretString),
    Send(LoginPlan),
}

pub(super) struct LoginPlan {
    pub(super) method: LoginMethod,
    pub(super) request: VaultRequest,
}

const APPROLE_LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const TLS_LOGIN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Serialize)]
struct AppRoleLoginBody<'borrowed> {
    role_id: &'borrowed str,
    secret_id: &'borrowed str,
}

#[derive(Serialize)]
struct TlsCertLoginBody<'borrowed> {
    name: Option<&'borrowed str>,
}

fn request_for<T>(
    manager: &HashicorpSecretManager<T>,
    method: LoginMethod,
    url: String,
    body: String,
    client_certificate: Option<ClientCertificate>,
) -> VaultRequest
where
    T: VaultTransport,
{
    let (timeout, follow_redirects) = match method {
        LoginMethod::AppRole => (APPROLE_LOGIN_TIMEOUT, true),
        LoginMethod::TlsCert => (TLS_LOGIN_TIMEOUT, false),
    };
    VaultRequest {
        method: VaultMethod::Post,
        url,
        token: None,
        namespace: manager.vault_login_namespace().map(str::to_owned),
        body: Some(SecretString::from(body)),
        client_certificate,
        timeout,
        follow_redirects,
        max_response_bytes: MAX_RESPONSE_BYTES,
    }
}

fn finish_login<T>(
    reply: Result<VaultResponse, T::Error>,
    method: LoginMethod,
    max_response_bytes: usize,
) -> Result<VaultLogin, VaultError<T::Error>>
where
    T: VaultTransport,
{
    let response = reply.map_err(|source| VaultError::Transport { method, source })?;
    if !(200..300).contains(&response.status) {
        return Err(VaultError::Status {
            method,
            status: response.status,
        });
    }
    if response.body.expose_secret().len() > max_response_bytes {
        return Err(VaultError::ResponseTooLarge {
            method,
            limit: max_response_bytes,
        });
    }
    decode_login_response(response.body.expose_secret().as_bytes())
        .map_err(|_source| VaultError::InvalidLogin { method })
}

pub(super) fn pending_login<T>(
    manager: &HashicorpSecretManager<T>,
) -> Result<PendingLogin, VaultError<T::Error>>
where
    T: VaultTransport,
{
    match &manager.settings.auth {
        VaultAuth::Token(token) => Ok(PendingLogin::Ready(token.clone())),
        VaultAuth::AppRole {
            role_id,
            secret_id,
            mount_path,
        } => approle_login(manager, role_id, secret_id, mount_path),
        VaultAuth::TlsCert {
            client_certificate,
            role,
        } => {
            tracing::debug!("Using TLS cert auth for Hashicorp Vault");
            let method = LoginMethod::TlsCert;
            let body = serde_json::to_string(&TlsCertLoginBody {
                name: role.as_deref(),
            })
            .map_err(|_source| VaultError::InvalidLogin { method })?;
            let url = format!("{}/v1/auth/cert/login", manager.vault_addr());
            let certificate = Some(client_certificate.clone());
            let request = request_for(manager, method, url, body, certificate);
            Ok(PendingLogin::Send(LoginPlan { method, request }))
        }
    }
}

fn approle_login<T>(
    manager: &HashicorpSecretManager<T>,
    role_id: &SecretString,
    secret_id: &SecretString,
    mount_path: &str,
) -> Result<PendingLogin, VaultError<T::Error>>
where
    T: VaultTransport,
{
    tracing::debug!("Using AppRole auth for Hashicorp Vault");
    if let Some(token) = cached_token(manager) {
        tracing::debug!("Using cached Vault token from AppRole auth");
        return Ok(PendingLogin::Ready(token));
    }
    let method = LoginMethod::AppRole;
    let body = serde_json::to_string(&AppRoleLoginBody {
        role_id: role_id.expose_secret(),
        secret_id: secret_id.expose_secret(),
    })
    .map_err(|_source| VaultError::InvalidLogin { method })?;
    let url = format!("{}/v1/auth/{mount_path}/login", manager.vault_addr());
    let request = request_for(manager, method, url, body, None);
    Ok(PendingLogin::Send(LoginPlan { method, request }))
}

pub(super) fn login_outcome<T>(
    manager: &HashicorpSecretManager<T>,
    method: LoginMethod,
    reply: Result<VaultResponse, T::Error>,
) -> Result<SecretString, VaultError<T::Error>>
where
    T: VaultTransport,
{
    let login = finish_login::<T>(reply, method, MAX_RESPONSE_BYTES)?;
    match method {
        LoginMethod::AppRole => {
            tracing::debug!(
                "Successfully obtained Vault token via AppRole auth. Lease duration: {}s",
                login.lease_duration
            );
            store_token(manager, login.client_token.clone(), login.lease_duration);
        }
        LoginMethod::TlsCert => {
            tracing::debug!("Successfully obtained Vault token via TLS cert auth.");
        }
    }
    Ok(login.client_token)
}

fn cached_token<T>(manager: &HashicorpSecretManager<T>) -> Option<SecretString> {
    let now = (manager.clock)();
    let slot = manager.token.lock().unwrap_or_else(PoisonError::into_inner);
    let cached = slot
        .as_ref()
        .filter(|entry| now <= entry.expiry)
        .map(|entry| entry.token.clone());
    drop(slot);
    cached
}

fn store_token<T>(manager: &HashicorpSecretManager<T>, token: SecretString, lease_seconds: u64) {
    let Some(expiry) = (manager.clock)().checked_add(Duration::from_secs(lease_seconds)) else {
        return;
    };
    let mut slot = manager.token.lock().unwrap_or_else(PoisonError::into_inner);
    *slot = Some(CachedToken { token, expiry });
}

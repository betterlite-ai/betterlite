use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use super::HashicorpSecretManager;
use super::error::{LoginMethod, VaultError};
use super::response::{VaultLogin, decode_login_response};
use super::transport::{
    MAX_RESPONSE_BYTES, VaultMethod, VaultRequest, VaultResponse, VaultTransport,
};
use crate::cyberark::transport::ClientCertificate;

mod tests;

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

#[derive(Clone, Copy)]
pub(super) enum LoginChannel<'borrowed> {
    AppRole {
        role_id: &'borrowed SecretString,
        secret_id: &'borrowed SecretString,
        mount_path: &'borrowed str,
    },
    TlsCert {
        client_certificate: &'borrowed ClientCertificate,
        role: Option<&'borrowed str>,
    },
}

pub(super) fn login_request<T>(
    manager: &HashicorpSecretManager<T>,
    channel: LoginChannel<'_>,
) -> Result<VaultRequest, VaultError<T::Error>>
where
    T: VaultTransport,
{
    match channel {
        LoginChannel::AppRole {
            role_id,
            secret_id,
            mount_path,
        } => {
            let body = serde_json::to_string(&AppRoleLoginBody {
                role_id: role_id.expose_secret(),
                secret_id: secret_id.expose_secret(),
            })
            .map_err(|_source| VaultError::InvalidLogin {
                method: LoginMethod::AppRole,
            })?;
            let url = format!("{}/v1/auth/{mount_path}/login", manager.vault_addr());
            Ok(request_for(manager, LoginMethod::AppRole, url, body, None))
        }
        LoginChannel::TlsCert {
            client_certificate,
            role,
        } => {
            let body =
                serde_json::to_string(&TlsCertLoginBody { name: role }).map_err(|_source| {
                    VaultError::InvalidLogin {
                        method: LoginMethod::TlsCert,
                    }
                })?;
            let url = format!("{}/v1/auth/cert/login", manager.vault_addr());
            Ok(request_for(
                manager,
                LoginMethod::TlsCert,
                url,
                body,
                Some(client_certificate.clone()),
            ))
        }
    }
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

pub(super) fn finish_login<T>(
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

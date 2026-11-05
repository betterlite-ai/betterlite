use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use secrecy::{ExposeSecret as _, SecretString};

use super::CyberArkSecretManager;
use super::error::CyberArkError;
use super::transport::{ConjurRequest, ConjurResponse, ConjurTransport, HttpMethod};

mod tests;

const TOKEN_KEY: &str = "cyberark_auth_token";

pub(super) async fn authorization<T>(
    manager: &CyberArkSecretManager<T>,
) -> Result<SecretString, CyberArkError<T::Error>>
where
    T: ConjurTransport,
{
    if let Some(header) = manager.tokens.get(TOKEN_KEY) {
        return Ok(header);
    }
    tracing::debug!("Authenticating with CyberArk Conjur...");
    let url = authentication_url(manager);
    let reply = manager
        .transport
        .send(authentication_request(manager, &url))
        .await;
    finish_authentication(manager, reply)
}

pub(super) fn authorization_blocking<T>(
    manager: &CyberArkSecretManager<T>,
) -> Result<SecretString, CyberArkError<T::Error>>
where
    T: ConjurTransport,
{
    if let Some(header) = manager.tokens.get(TOKEN_KEY) {
        return Ok(header);
    }
    tracing::debug!("Authenticating with CyberArk Conjur...");
    let url = authentication_url(manager);
    let reply = manager
        .transport
        .send_blocking(authentication_request(manager, &url));
    finish_authentication(manager, reply)
}

fn authentication_url<T>(manager: &CyberArkSecretManager<T>) -> String {
    let settings = &manager.settings;
    format!(
        "{}/authn/{}/{}/authenticate",
        settings.api_base, settings.account, settings.username
    )
}

const fn authentication_request<'borrowed, T>(
    manager: &'borrowed CyberArkSecretManager<T>,
    url: &'borrowed str,
) -> ConjurRequest<'borrowed> {
    ConjurRequest {
        method: HttpMethod::Post,
        url,
        authorization: None,
        content_type: None,
        body: Some(&manager.settings.api_key),
        verify_tls: manager.settings.verify_tls,
        client_certificate: manager.settings.client_certificate.as_ref(),
    }
}

fn finish_authentication<T>(
    manager: &CyberArkSecretManager<T>,
    reply: Result<ConjurResponse, T::Error>,
) -> Result<SecretString, CyberArkError<T::Error>>
where
    T: ConjurTransport,
{
    let response = reply
        .map_err(CyberArkError::Transport)
        .and_then(ConjurResponse::require_success)
        .map_err(|source| CyberArkError::Authentication(Box::new(source)))?;
    let token = STANDARD.encode(response.body.expose_secret());
    tracing::debug!("Successfully authenticated with CyberArk Conjur.");
    let header = SecretString::from(format!("Token token=\"{token}\""));
    manager.tokens.set(TOKEN_KEY, header.clone());
    Ok(header)
}

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use secrecy::{ExposeSecret as _, SecretString};

use super::CyberArkSecretManager;
use super::error::CyberArkError;
use super::transport::{ConjurResponse, ConjurTransport};

mod tests;

pub(super) const TOKEN_KEY: &str = "cyberark_auth_token";

pub(super) fn begin_login<T>(manager: &CyberArkSecretManager<T>) -> String {
    tracing::debug!("Authenticating with CyberArk Conjur...");
    let settings = &manager.settings;
    format!(
        "{}/authn/{}/{}/authenticate",
        settings.api_base, settings.account, settings.username
    )
}

pub(super) fn finish_login<T>(
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

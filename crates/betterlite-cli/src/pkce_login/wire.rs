use secrecy::{ExposeSecret as _, SecretString};

use super::{HttpResponse, models::PkceFailure};

#[derive(serde::Deserialize)]
pub(super) struct RegisteredClient {
    pub(super) client_id: String,
}

#[derive(serde::Serialize)]
pub(super) struct ClientRegistration {
    pub(super) client_name: &'static str,
    pub(super) redirect_uris: Vec<String>,
    pub(super) grant_types: [&'static str; 2],
    pub(super) response_types: [&'static str; 1],
    pub(super) token_endpoint_auth_method: &'static str,
}

#[derive(serde::Deserialize)]
pub(super) struct TokenResponse {
    pub(super) access_token: SecretString,
    pub(super) expires_in: i64,
    pub(super) refresh_token: SecretString,
    pub(super) user_id: Option<String>,
    pub(super) team_id: Option<String>,
}

pub(super) fn error_detail(response: &HttpResponse) -> String {
    let fallback = String::from_utf8_lossy(&response.body);
    let detail = serde_json::from_slice::<ErrorBody>(&response.body)
        .ok()
        .and_then(|body| body.error_description.or(body.error).or(body.detail))
        .unwrap_or_else(|| fallback.into_owned());
    detail.chars().take(200).collect()
}

#[derive(serde::Deserialize)]
struct ErrorBody {
    error_description: Option<String>,
    error: Option<String>,
    detail: Option<String>,
}

pub(super) fn parse_token(response: &HttpResponse) -> Result<TokenResponse, PkceFailure> {
    let token: TokenResponse = serde_json::from_slice(&response.body)
        .map_err(|source| PkceFailure::TokenResponse { source })?;
    if token.access_token.expose_secret().is_empty()
        || token.refresh_token.expose_secret().is_empty()
        || token.expires_in <= 0
    {
        return Err(PkceFailure::Protocol(
            "invalid token response fields".to_owned(),
        ));
    }
    Ok(token)
}

pub(super) fn refuse_redirect(name: &str, response: &HttpResponse) -> Result<(), PkceFailure> {
    if response.status == 307 || response.status == 308 {
        return Err(PkceFailure::Protocol(format!(
            "{name} redirected; refusing to follow it"
        )));
    }
    Ok(())
}

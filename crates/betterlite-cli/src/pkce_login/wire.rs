use std::str::from_utf8;

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
    pub(super) access_token: String,
    pub(super) expires_in: i64,
    pub(super) refresh_token: String,
    pub(super) user_id: Option<String>,
    pub(super) team_id: Option<String>,
}

pub(super) fn error_detail(response: &HttpResponse) -> String {
    let fallback = from_utf8(&response.body).unwrap_or("");
    let detail = serde_json::from_slice::<ErrorBody>(&response.body)
        .ok()
        .and_then(|body| body.error_description.or(body.error).or(body.detail))
        .unwrap_or_else(|| String::from(fallback));
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
        .map_err(|error| PkceFailure::new_owned(format!("invalid token response: {error}")))?;
    if token.access_token.is_empty() || token.refresh_token.is_empty() || token.expires_in <= 0 {
        return Err(PkceFailure::new("invalid token response fields"));
    }
    Ok(token)
}

pub(super) fn refuse_redirect(name: &str, response: &HttpResponse) -> Result<(), PkceFailure> {
    if response.status == 307 || response.status == 308 {
        return Err(PkceFailure::new_owned(format!(
            "{name} redirected; refusing to follow it"
        )));
    }
    Ok(())
}

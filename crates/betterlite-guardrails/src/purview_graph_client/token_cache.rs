use std::sync::PoisonError;
use std::time::{Duration, SystemTime};

use betterlite_http::proxy::limits::read_bounded;
use reqwest::header::CONTENT_TYPE;
use secrecy::{ExposeSecret as _, SecretString};
use url::form_urlencoded;

use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::graph_api::MAX_RESPONSE_BYTES;
use crate::purview_graph_client::wire::GraphTokenResponse;
use crate::purview_graph_client::{GRAPH_SCOPE, PurviewGuardrailBase, TOKEN_EXPIRY_MARGIN};

#[derive(Debug)]
pub struct CachedToken {
    pub access_token: SecretString,
    pub expires_at: SystemTime,
}

fn cached_token(base: &PurviewGuardrailBase, now: SystemTime) -> Option<SecretString> {
    let threshold = now.checked_add(TOKEN_EXPIRY_MARGIN)?;
    base.scope_and_token_cache_lock
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .oauth_token_cache
        .as_ref()
        .filter(|token| token.expires_at > threshold)
        .map(|token| token.access_token.clone())
}

async fn request_token(
    base: &PurviewGuardrailBase,
) -> Result<GraphTokenResponse, PurviewGraphError> {
    let url = format!(
        "{}/{}/oauth2/v2.0/token",
        base.login_authority, base.tenant_id
    );
    let form = form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "client_credentials")
        .append_pair("client_id", &base.client_id)
        .append_pair("client_secret", base.client_secret.expose_secret())
        .append_pair("scope", GRAPH_SCOPE)
        .finish();
    let mut response = base
        .client
        .post(url)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form)
        .timeout(base.timeout)
        .send()
        .await
        .map_err(|source| PurviewGraphError::Request { source })?;
    let status = response.status();
    if !status.is_success() {
        return Err(PurviewGraphError::TokenStatus { status });
    }
    let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
        .await
        .map_err(|source| PurviewGraphError::Body { source })?;
    serde_json::from_slice(&body).map_err(|source| PurviewGraphError::ResponseJson { source })
}

pub async fn get_access_token(
    base: &PurviewGuardrailBase,
) -> Result<SecretString, PurviewGraphError> {
    if let Some(token) = cached_token(base, (base.clock)()) {
        return Ok(token);
    }
    let token_data = request_token(base).await?;
    let access_token = SecretString::from(token_data.access_token);
    // The expiry counts from when the token arrived, not from when the request started.
    let expires_at = (base.clock)()
        .checked_add(Duration::from_secs(token_data.expires_in))
        .ok_or(PurviewGraphError::TokenExpiry)?;
    base.scope_and_token_cache_lock
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .oauth_token_cache = Some(CachedToken {
        access_token: access_token.clone(),
        expires_at,
    });
    tracing::debug!(
        expires_in = token_data.expires_in,
        "Purview: acquired new OAuth2 token"
    );
    Ok(access_token)
}

mod tests;

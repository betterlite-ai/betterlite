use std::collections::BTreeMap;
use std::fmt::Display;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use crate::mcp_stored_contracts::JsonValue;
use crate::mcp_user_credentials::rows::CredentialRows;
use crate::mcp_user_credentials::store::get_user_oauth_credential;
use crate::mcp_user_credentials::store_user_oauth_credential;
use crate::mcp_user_credentials::types::{OAuthCredentialPayload, StoreOAuthCredential};

use super::ports::{
    TokenEndpointPost, TokenRequestBuilder, TokenResponse, TokenServer, TokenServices,
};
use super::{OauthBindingPort, TokenResolutionError};

const REFRESH_GRANT: &str = "refresh_token";

pub(super) struct TokenExchange {
    pub(super) body: TokenResponse,
    pub(super) identity_binding_proof: Option<String>,
}

pub(super) struct RefreshedToken {
    pub(super) access_token: SecretString,
    pub(super) refresh_token: Option<SecretString>,
    pub(super) expires_in: Option<i64>,
    pub(super) scopes: Option<Vec<String>>,
    pub(super) identity_binding_proof: Option<String>,
}

// LiteLLM's `if not refresh_token`: an absent or empty stored token cannot mint a new one.
pub(super) fn stored_refresh_token<'cred>(
    cred: &'cred OAuthCredentialPayload,
    user_id: &str,
    server_id: &str,
) -> Option<&'cred SecretString> {
    let refresh_token = cred
        .refresh_token
        .as_ref()
        .filter(|token| !token.expose_secret().is_empty());
    if refresh_token.is_none() {
        tracing::debug!(
            user_id,
            server_id,
            "refresh_user_oauth_token: no refresh_token stored"
        );
    }
    refresh_token
}

// LiteLLM's effective_token_url or token_url, with the falsy empty string meaning the same as absent.
pub(super) fn token_url<'server, S>(server: &'server S, server_id: &str) -> Option<&'server str>
where
    S: TokenServer,
{
    let token_url = server.effective_token_url().filter(|url| !url.is_empty());
    if token_url.is_none() {
        tracing::debug!(
            server_id,
            "refresh_user_oauth_token: server has no token_url configured"
        );
    }
    token_url
}

// LiteLLM's one try block: the request build, the POST and the JSON body, each a warning and None on failure.
pub(super) async fn token_response<A, P, B, H>(
    services: &TokenServices<'_, A, P, B, H>,
    server: &P::Server,
    token_url: &str,
    refresh_token: &SecretString,
    user_id: &str,
) -> Option<TokenExchange>
where
    P: OauthBindingPort,
    P::Server: TokenServer,
    B: TokenRequestBuilder<P::Server>,
    H: TokenEndpointPost,
{
    let server_id = server.server_id();
    let request = match services.builder.token_request(server) {
        Ok(request) => request,
        Err(error) => {
            warn_request_failed(user_id, server_id, &error);
            return None;
        }
    };
    let headers = accept_headers(&request.headers);
    let form = form_body(&request.body, refresh_token);
    let body = match services.post.post_token(token_url, &headers, &form).await {
        Ok(body) => body,
        Err(error) => {
            warn_request_failed(user_id, server_id, &error);
            return None;
        }
    };
    enforce_exchange(services, server, user_id, refresh_token, body).await
}

// LiteLLM's enforce_oauth_identity_binding call: the presented refresh token proves ownership, and a denial reads as no token.
async fn enforce_exchange<A, P, B, H>(
    services: &TokenServices<'_, A, P, B, H>,
    server: &P::Server,
    user_id: &str,
    refresh_token: &SecretString,
    body: TokenResponse,
) -> Option<TokenExchange>
where
    P: OauthBindingPort,
{
    let ownership = P::Ownership::from(refresh_token.clone());
    let enforced = services
        .binding
        .enforce(
            server,
            &body,
            Some(user_id),
            REFRESH_GRANT,
            Some(&ownership),
        )
        .await;
    match enforced {
        Ok(identity_binding_proof) => Some(TokenExchange {
            body,
            identity_binding_proof,
        }),
        Err(_denied) => None,
    }
}

fn warn_request_failed<E>(user_id: &str, server_id: &str, error: &E)
where
    E: Display,
{
    tracing::warn!(
        user_id,
        server_id,
        error = %error,
        "refresh_user_oauth_token: refresh request failed"
    );
}

// `{"Accept": "application/json", **token_request.headers}`: the builder's own headers win on a repeated name.
fn accept_headers(headers: &BTreeMap<String, SecretString>) -> BTreeMap<String, SecretString> {
    let mut all = BTreeMap::from([("Accept".to_owned(), SecretString::from("application/json"))]);
    all.extend(headers.clone());
    all
}

// `{"grant_type": "refresh_token", "refresh_token": ..., **token_request.body}`: the builder's body wins on a repeated key.
fn form_body(
    body: &BTreeMap<String, SecretString>,
    refresh_token: &SecretString,
) -> BTreeMap<String, SecretString> {
    let mut form = BTreeMap::from([
        ("grant_type".to_owned(), SecretString::from(REFRESH_GRANT)),
        ("refresh_token".to_owned(), refresh_token.clone()),
    ]);
    form.extend(body.clone());
    form
}

// LiteLLM's `if not access_token`: absent, empty and a non-string are all a response without a token.
pub(super) fn access_token(
    body: &TokenResponse,
    user_id: &str,
    server_id: &str,
) -> Option<SecretString> {
    let token = non_empty_text(body, "access_token");
    if token.is_none() {
        tracing::warn!(
            user_id,
            server_id,
            "refresh_user_oauth_token: token response missing access_token"
        );
    }
    token.map(SecretString::from)
}

fn non_empty_text<'body>(body: &'body TokenResponse, field: &str) -> Option<&'body str> {
    match body.get(field) {
        Some(JsonValue::String(text)) if !text.is_empty() => Some(text.as_str()),
        Some(
            JsonValue::String(_)
            | JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}

// A provider that rotates its refresh token replaces the stored one; one that omits it keeps the presented token.
pub(super) fn refreshed_token(
    exchange: &TokenExchange,
    cred: &OAuthCredentialPayload,
    access_token: SecretString,
) -> RefreshedToken {
    let rotated = non_empty_text(&exchange.body, "refresh_token").map(SecretString::from);
    RefreshedToken {
        access_token,
        refresh_token: rotated.or_else(|| cred.refresh_token.clone()),
        expires_in: expires_in(&exchange.body),
        scopes: scopes(&exchange.body, cred),
        identity_binding_proof: exchange.identity_binding_proof.clone(),
    }
}

// LiteLLM's int(raw_expires) under a TypeError/ValueError catch: a non-integral answer reads as no expiry.
fn expires_in(body: &TokenResponse) -> Option<i64> {
    match body.get("expires_in") {
        Some(JsonValue::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().and_then(whole_number)),
        Some(JsonValue::String(text)) => text.trim().parse::<i64>().ok(),
        Some(JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_))
        | None => None,
    }
}

// Python's int() on a float truncates toward zero; a value outside i64 has no bounded equivalent.
fn whole_number(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    value.trunc().to_string().parse::<i64>().ok()
}

// An omitted scope means unchanged (RFC 6749 5.1), so the stored grant carries over; a whitespace-only scope is the falsy empty answer.
fn scopes(body: &TokenResponse, cred: &OAuthCredentialPayload) -> Option<Vec<String>> {
    let granted = non_empty_text(body, "scope")
        .map(|scope| {
            scope
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<String>>()
        })
        .filter(|scopes| !scopes.is_empty());
    granted.or_else(|| cred.scopes.clone())
}

// LiteLLM's store under skip_byok_guard=True: the row is already OAuth2, so the find_unique guard is skipped; the stored payload is read back.
pub(super) async fn store<A, P, B, H>(
    services: &TokenServices<'_, A, P, B, H>,
    user_id: &str,
    server_id: &str,
    refreshed: &RefreshedToken,
    now: DateTime<Utc>,
) -> Result<Option<OAuthCredentialPayload>, TokenResolutionError>
where
    A: CredentialRows,
{
    let request = StoreOAuthCredential {
        user_id,
        server_id,
        access_token: &refreshed.access_token,
        refresh_token: refreshed.refresh_token.as_ref(),
        expires_in: refreshed.expires_in,
        scopes: refreshed.scopes.as_deref(),
        skip_byok_guard: true,
        identity_binding_proof: refreshed.identity_binding_proof.as_deref(),
    };
    store_user_oauth_credential(services.store, services.cipher, &request, now).await?;
    tracing::info!(
        user_id,
        server_id,
        "refresh_user_oauth_token: refreshed token"
    );
    Ok(get_user_oauth_credential(services.store, services.cipher, user_id, server_id).await?)
}

// LiteLLM's `if not refreshed or not refreshed.get("access_token")`, as the resolve path's last check.
pub(super) fn usable(credential: Option<OAuthCredentialPayload>) -> Option<OAuthCredentialPayload> {
    credential.filter(|cred| {
        cred.access_token
            .as_ref()
            .is_some_and(|token| !token.expose_secret().is_empty())
    })
}

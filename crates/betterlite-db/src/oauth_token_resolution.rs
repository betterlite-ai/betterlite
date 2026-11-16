use std::error::Error as StdError;
use std::future::Future;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::managed_resource_rows::mcp::McpServerTable;
use crate::mcp::table_access::types::McpTableError;
use crate::mcp_user_credentials::oauth_grant_state;
use crate::mcp_user_credentials::rows::{CredentialRows, CredentialScope};
use crate::mcp_user_credentials::types::{
    CredentialError, OAuthCredentialPayload, OAuthGrantState,
};
use crate::stored_cipher::DbCipher;

pub mod identity;
pub mod ports;
pub mod purge;

mod refresh;
mod resolve;
mod tests;

use ports::{
    AccessTokenRequest, PerUserTokenCache, PerUserTokenTtl, TokenEndpointPost, TokenRequestBuilder,
    TokenServer, TokenServices,
};

// LiteLLM's MCP_PER_USER_TOKEN_EXPIRY_BUFFER_SECONDS default; the caller passes its configured buffer through TokenServices.
pub const TOKEN_EXPIRY_BUFFER_SECONDS: i64 = 60;

#[derive(Debug, Error)]
pub enum TokenResolutionError {
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    Table(#[from] McpTableError),
}

// The arguments of LiteLLM's enforce_oauth_identity_binding, as one value so the trait method stays under the argument limit.
pub struct EnforceRequest<'request, S, O> {
    pub server: &'request S,
    pub token_response: &'request ports::TokenResponse,
    pub user_id: Option<&'request str>,
    pub grant_type: &'request str,
    pub ownership: Option<&'request O>,
}

// LiteLLM's enforce_oauth_identity_binding and credential_binding_matches, which B4659 implements in betterlite-mcp.
pub trait OauthBindingPort: Send + Sync {
    type Server: Send + Sync;
    type Ownership: From<SecretString> + Send + Sync;
    type Denied: Send + Sync;

    fn enforce(
        &self,
        request: &EnforceRequest<'_, Self::Server, Self::Ownership>,
    ) -> impl Future<Output = Result<Option<String>, Self::Denied>> + Send;

    fn proof_matches(
        &self,
        server: &Self::Server,
        user_id: &str,
        stored_proof: Option<&str>,
    ) -> impl Future<Output = bool> + Send;
}

// The manager's invalidate_user_oauth_token_cache: the discovery lists, the v2 store and the legacy per-user cache, all best-effort.
pub trait UserTokenCaches: Send + Sync {
    type CacheError: StdError + Send + 'static;

    fn invalidate_discovery_lists(&self, server_id: &str);

    fn invalidate_oauth_store(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<(), Self::CacheError>> + Send;

    fn delete_legacy(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<(), Self::CacheError>> + Send;
}

// LiteLLM's mcp_oauth_token_identity: the 12 fields whose change makes a stored per-user token stale, client credentials decrypted.
#[must_use]
pub fn mcp_oauth_token_identity(
    server: &McpServerTable,
    cipher: &DbCipher,
) -> identity::McpOauthTokenIdentity {
    let creds = identity::credential_blob(server);
    identity::McpOauthTokenIdentity {
        url: server.url.clone(),
        spec_path: server.spec_path.clone(),
        auth_type: server.auth_type,
        oauth2_flow: server.oauth2_flow,
        issuer: server.issuer.clone(),
        authorization_url: server.authorization_url.clone(),
        token_url: server.token_url.clone(),
        registration_url: server.registration_url.clone(),
        client_id: match identity::decrypted_credential_field(&creds, "client_id", cipher) {
            identity::DecryptedField::Text(secret) => Some(secret.expose_secret().to_owned()),
            identity::DecryptedField::Value(_) => None,
        },
        client_secret: match identity::decrypted_credential_field(&creds, "client_secret", cipher) {
            identity::DecryptedField::Text(secret) => Some(secret),
            identity::DecryptedField::Value(_) => None,
        },
        scopes: server
            .credentials
            .as_ref()
            .and_then(|credentials| credentials.scopes.clone())
            .flatten(),
        upstream_resource: server
            .credentials
            .as_ref()
            .and_then(|credentials| credentials.upstream_resource.clone())
            .flatten(),
    }
}

// LiteLLM's purge_user_oauth_credentials_for_server: only decoded OAuth2 rows go, and every purged user's caches drop with them.
pub async fn purge_server_oauth_credentials<A, C>(
    table: &A,
    cipher: &DbCipher,
    caches: &C,
    server_id: &str,
) -> Result<usize, TokenResolutionError>
where
    A: purge::PurgeRows,
    C: UserTokenCaches,
{
    let rows = table
        .find_credentials(CredentialScope::Server(server_id))
        .await?;
    let users = purge::oauth_users(&rows, cipher)?;
    if users.is_empty() {
        return Ok(0);
    }
    let deleted = table.delete_credential_pairs(server_id, &users).await?;
    for user_id in &users {
        invalidate_user_token_cache(caches, user_id, server_id).await;
    }
    if deleted != users.len() {
        tracing::warn!(
            "MCP server {server_id}: purge removed {deleted} OAuth credential row(s) but {} were enumerated; row(s) were deleted concurrently during the purge",
            users.len()
        );
    }
    Ok(deleted)
}

// LiteLLM's invalidate_user_oauth_token_cache: each cache drop is logged on failure and never returned, and the legacy one still runs after the v2 store fails.
pub async fn invalidate_user_token_cache<C>(caches: &C, user_id: &str, server_id: &str)
where
    C: UserTokenCaches,
{
    caches.invalidate_discovery_lists(server_id);
    if let Err(error) = caches.invalidate_oauth_store(user_id, server_id).await {
        tracing::warn!(
            "Failed to invalidate cached MCP OAuth token for user={user_id} server={server_id}: {error}"
        );
    }
    if let Err(error) = caches.delete_legacy(user_id, server_id).await {
        tracing::warn!(
            "Failed to drop legacy cached MCP OAuth token for user={user_id} server={server_id}: {error}"
        );
    }
}

// LiteLLM's refresh_user_oauth_token: proof, token_url, POST, binding and store, each failure reading as no token.
pub async fn refresh_user_oauth_token<A, P, B, H>(
    services: &TokenServices<'_, A, P, B, H>,
    server: &P::Server,
    user_id: &str,
    cred: &OAuthCredentialPayload,
    now: DateTime<Utc>,
) -> Result<Option<OAuthCredentialPayload>, TokenResolutionError>
where
    A: CredentialRows,
    P: OauthBindingPort,
    P::Server: TokenServer,
    B: TokenRequestBuilder<P::Server>,
    H: TokenEndpointPost,
{
    let server_id = server.server_id();
    if server.binding_enforced()
        && !services
            .binding
            .proof_matches(server, user_id, cred.identity_binding_proof.as_deref())
            .await
    {
        return Ok(None);
    }
    let Some(refresh_token) = refresh::stored_refresh_token(cred, user_id, server_id) else {
        return Ok(None);
    };
    let Some(token_url) = refresh::token_url(server, server_id) else {
        return Ok(None);
    };
    let Some(exchange) =
        refresh::token_response(services, server, token_url, refresh_token, user_id).await
    else {
        return Ok(None);
    };
    let Some(access_token) = refresh::access_token(&exchange.body, user_id, server_id) else {
        return Ok(None);
    };
    let refreshed = refresh::refreshed_token(&exchange, cred, access_token);
    refresh::store(services, user_id, server_id, &refreshed, now).await
}

// LiteLLM's resolve_valid_user_oauth_token: binding first, then the grant state, and the store is only touched by a refresh.
pub async fn resolve_valid_user_oauth_token<A, P, B, H>(
    services: &TokenServices<'_, A, P, B, H>,
    server: &P::Server,
    user_id: &str,
    stored: Option<&OAuthCredentialPayload>,
    now: DateTime<Utc>,
) -> Result<Option<OAuthCredentialPayload>, TokenResolutionError>
where
    A: CredentialRows,
    P: OauthBindingPort,
    P::Server: TokenServer,
    B: TokenRequestBuilder<P::Server>,
    H: TokenEndpointPost,
{
    let Some(cred) = stored else {
        return Ok(None);
    };
    let grant = oauth_grant_state(Some(cred), now, services.buffer_seconds);
    if grant == OAuthGrantState::Absent {
        return Ok(None);
    }
    if server.binding_enforced()
        && !services
            .binding
            .proof_matches(server, user_id, cred.identity_binding_proof.as_deref())
            .await
    {
        return Ok(None);
    }
    if grant == OAuthGrantState::Valid {
        return Ok(Some(cred.clone()));
    }
    let refreshed = refresh_user_oauth_token(services, server, user_id, cred, now).await?;
    Ok(refresh::usable(refreshed))
}

// LiteLLM's resolve_user_oauth_access_token: Redis first, then the DB and a refresh, warming the cache only when no prefetch was supplied.
pub async fn resolve_user_access_token<A, P, B, H, C, T>(
    services: &TokenServices<'_, A, P, B, H>,
    cache: &C,
    ttl: &T,
    request: &AccessTokenRequest<'_, P::Server>,
) -> Option<SecretString>
where
    A: CredentialRows,
    P: OauthBindingPort,
    P::Server: TokenServer,
    B: TokenRequestBuilder<P::Server>,
    H: TokenEndpointPost,
    C: PerUserTokenCache,
    T: PerUserTokenTtl<P::Server>,
{
    let user_id = request.user_id?;
    let server_id = request.server.server_id();
    if user_id.is_empty() || server_id.is_empty() {
        return None;
    }
    if request.prefetched.is_none()
        && let Some(token) =
            resolve::cached(services, cache, request.server, user_id, server_id).await
    {
        return Some(token);
    }
    resolve::resolved(services, cache, ttl, request, user_id).await
}

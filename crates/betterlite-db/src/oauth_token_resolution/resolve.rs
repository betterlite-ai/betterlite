use std::fmt::Display;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use betterlite_models::savings_evidence::billed_at::decode_billed_at;

use crate::mcp_user_credentials::rows::CredentialRows;
use crate::mcp_user_credentials::store::get_user_oauth_credential;
use crate::mcp_user_credentials::types::OAuthCredentialPayload;

use super::ports::{
    AccessTokenRequest, CachedUserToken, PerUserTokenCache, PerUserTokenTtl, TokenEndpointPost,
    TokenRequestBuilder, TokenServer, TokenServices,
};
use super::{OauthBindingPort, resolve_valid_user_oauth_token};

// The Redis fast path: a non-binding server reads the plain token, a binding-enforced one proves the cached token before returning it.
pub(super) async fn cached<A, P, B, H, C>(
    services: &TokenServices<'_, A, P, B, H>,
    cache: &C,
    server: &P::Server,
    user_id: &str,
    server_id: &str,
) -> Option<SecretString>
where
    P: OauthBindingPort,
    P::Server: TokenServer,
    C: PerUserTokenCache,
{
    if !server.binding_enforced() {
        return cache.get(user_id, server_id).await;
    }
    let token = cache.get_token(user_id, server_id).await?;
    if services
        .binding
        .proof_matches(server, user_id, token.identity_binding_proof.as_deref())
        .await
    {
        return Some(token.access_token);
    }
    cache.delete(user_id, server_id).await;
    None
}

// LiteLLM's DB read under the same try as the cache: a prefetched map replaces it and `not cred or not cred.get("access_token")` reads as not authorized.
pub(super) async fn credential<A, P, B, H>(
    services: &TokenServices<'_, A, P, B, H>,
    request: &AccessTokenRequest<'_, P::Server>,
    user_id: &str,
    server_id: &str,
) -> Option<OAuthCredentialPayload>
where
    A: CredentialRows,
    P: OauthBindingPort,
{
    let loaded = match request.prefetched {
        Some(prefetched) => prefetched.get(server_id).cloned(),
        None => {
            match get_user_oauth_credential(services.store, services.cipher, user_id, server_id)
                .await
            {
                Ok(credential) => credential,
                Err(error) => {
                    warn_resolution_failed(user_id, server_id, &error);
                    return None;
                }
            }
        }
    };
    let credential = loaded?;
    credential
        .access_token
        .as_ref()
        .filter(|token| !token.expose_secret().is_empty())?;
    Some(credential)
}

// The outer except's warning: LiteLLM logs the failure and the caller reads no token.
fn warn_resolution_failed<E>(user_id: &str, server_id: &str, error: &E)
where
    E: Display,
{
    tracing::warn!(
        "resolve_user_oauth_access_token: failed for user={user_id} server={server_id}: {error}"
    );
}

// resolve_valid, then the failure delete, the token and the cache warm; the caller passes the user id it already resolved.
pub(super) async fn resolved<A, P, B, H, C, T>(
    services: &TokenServices<'_, A, P, B, H>,
    cache: &C,
    ttl: &T,
    request: &AccessTokenRequest<'_, P::Server>,
    user_id: &str,
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
    let server_id = request.server.server_id();
    let cred = credential(services, request, user_id, server_id).await?;
    let resolved = match resolve_valid_user_oauth_token(
        services,
        request.server,
        user_id,
        Some(&cred),
        request.now,
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(error) => {
            warn_resolution_failed(user_id, server_id, &error);
            return None;
        }
    };
    let Some(resolved) = resolved else {
        cache.delete(user_id, server_id).await;
        return None;
    };
    let token = token(&resolved)?;
    if request.prefetched.is_none() {
        let remaining = remaining_token_seconds(resolved.expires_at.as_deref(), request.now);
        let ttl_seconds = ttl.ttl_seconds(request.server, remaining);
        cache.set(user_id, server_id, &token, ttl_seconds).await;
    }
    Some(token.access_token)
}

pub(super) fn token(cred: &OAuthCredentialPayload) -> Option<CachedUserToken> {
    let access_token = cred.access_token.clone()?;
    Some(CachedUserToken {
        access_token,
        identity_binding_proof: cred.identity_binding_proof.clone(),
    })
}

// LiteLLM's _remaining_token_seconds: absent, past and unparsable all read as no lifetime, and a naive timestamp is UTC.
pub(super) fn remaining_token_seconds(expires_at: Option<&str>, now: DateTime<Utc>) -> Option<i64> {
    let expires = decode_billed_at(expires_at.filter(|text| !text.is_empty())?)?;
    let remaining = expires.signed_duration_since(now).num_seconds();
    (remaining > 0).then_some(remaining)
}

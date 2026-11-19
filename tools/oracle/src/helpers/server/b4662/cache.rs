use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::mcp::oauth_caches::{MCPOAuth2TokenCache, MCPPerUserTokenCache};
use betterlite_server::mcp::outbound_credentials::PER_USER_TOKEN_KEY_PREFIX;
use betterlite_server::mcp::outbound_credentials::client_credentials::token_post::M2mTokenEndpointPost;

use super::support::{
    self, FakeBackend, FixedClock, OutcomeSpec, ScriptedPost, ServerSpec, ServersInput,
    authorization,
};

#[derive(Debug, Deserialize)]
pub(super) struct GetTokenInput {
    server: ServerSpec,
    #[serde(default)]
    outcomes: Vec<OutcomeSpec>,
    calls: usize,
    concurrent: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct FetchInput {
    server: ServerSpec,
    #[serde(default)]
    outcomes: Vec<OutcomeSpec>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InvalidateInput {
    server: ServerSpec,
    #[serde(default)]
    outcomes: Vec<OutcomeSpec>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PerUserGetInput {
    user_id: String,
    server_id: String,
    seed: Option<String>,
    garbage: bool,
    fail_get: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct PerUserSetInput {
    user_id: String,
    server_id: String,
    access_token: String,
    ttl: i64,
    identity_binding_proof: Option<String>,
    fail_set: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct PerUserDeleteInput {
    user_id: String,
    server_id: String,
    fail_delete: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct PerUserTtlInput {
    pub(super) server: ServerSpec,
    pub(super) expires_in: Option<i64>,
}

type PerUserCache = MCPPerUserTokenCache<FakeBackend>;

fn oauth2_cache(
    outcomes: &[OutcomeSpec],
) -> Result<(Arc<ScriptedPost>, MCPOAuth2TokenCache), Box<HelperError>> {
    let scripted = Arc::new(ScriptedPost::new(outcomes)?);
    let poster: Arc<dyn M2mTokenEndpointPost> = scripted.clone();
    Ok((
        scripted,
        MCPOAuth2TokenCache::new(poster, Arc::new(FixedClock)),
    ))
}

pub(super) fn identity(input: &ServersInput) -> Result<Value, Box<HelperError>> {
    let mut identities = Vec::with_capacity(input.servers.len());
    for spec in &input.servers {
        let server: MCPServer = serde_json::from_value(spec.0.clone())
            .map_err(|error| support::value_error(error.to_string()))?;
        identities.push(MCPOAuth2TokenCache::token_identity(&server));
    }
    Ok(json!({"identities": identities}))
}

pub(super) async fn get_token(input: &GetTokenInput) -> Result<Value, Box<HelperError>> {
    let server: MCPServer = serde_json::from_value(input.server.0.clone())
        .map_err(|error| support::value_error(error.to_string()))?;
    let (scripted, cache) = oauth2_cache(&input.outcomes)?;
    let futures = (0..input.calls).map(|_call| cache.async_get_token(&server));
    let mut tokens = Vec::with_capacity(input.calls);
    if input.concurrent {
        for result in futures_util::future::join_all(futures).await {
            tokens.push(result.map_err(|error| support::value_error(error.to_string()))?);
        }
    } else {
        for result in futures {
            tokens.push(
                result
                    .await
                    .map_err(|error| support::value_error(error.to_string()))?,
            );
        }
    }
    Ok(json!({"tokens": tokens, "posts": scripted.post_count()}))
}

pub(super) async fn fetch(input: &FetchInput) -> Result<Value, Box<HelperError>> {
    let server: MCPServer = serde_json::from_value(input.server.0.clone())
        .map_err(|error| support::value_error(error.to_string()))?;
    let (scripted, cache) = oauth2_cache(&input.outcomes)?;
    let (token, ttl) = cache
        .fetch_token(&server)
        .await
        .map_err(|error| support::value_error(error.to_string()))?;
    let call = scripted
        .first_call()
        .ok_or_else(|| support::value_error("the token request was not posted".to_owned()))?;
    let mut form = call.form;
    form.sort();
    Ok(json!({
        "token": token,
        "ttl": ttl,
        "form": form,
        "authorization": authorization(&call.headers),
    }))
}

pub(super) async fn invalidate(input: &InvalidateInput) -> Result<Value, Box<HelperError>> {
    let server: MCPServer = serde_json::from_value(input.server.0.clone())
        .map_err(|error| support::value_error(error.to_string()))?;
    let (scripted, cache) = oauth2_cache(&input.outcomes)?;
    let first = cache
        .async_get_token(&server)
        .await
        .map_err(|error| support::value_error(error.to_string()))?;
    cache.invalidate(&server.fields.server_id);
    let second = cache
        .async_get_token(&server)
        .await
        .map_err(|error| support::value_error(error.to_string()))?;
    Ok(json!({"tokens": [first, second], "posts": scripted.post_count()}))
}

pub(super) async fn per_user_get(input: &PerUserGetInput) -> Result<Value, Box<HelperError>> {
    let cache = PerUserCache {
        backend: FakeBackend::new(input.fail_get, false, false),
        codec: support::codec(),
    };
    let key = format!(
        "{PER_USER_TOKEN_KEY_PREFIX}{}:{}",
        input.user_id, input.server_id
    );
    if let Some(seed) = &input.seed {
        cache
            .set(&input.user_id, &input.server_id, seed, 300, None)
            .await;
    }
    if input.garbage {
        cache.backend.seed(&key, "garbage");
    }
    let token = cache
        .get_token(&input.user_id, &input.server_id)
        .await
        .map(|token| token.access_token);
    Ok(json!({"token": token}))
}

pub(super) async fn per_user_set(input: &PerUserSetInput) -> Result<Value, Box<HelperError>> {
    let cache = PerUserCache {
        backend: FakeBackend::new(false, input.fail_set, false),
        codec: support::codec(),
    };
    cache
        .set(
            &input.user_id,
            &input.server_id,
            &input.access_token,
            input.ttl,
            input.identity_binding_proof.as_deref(),
        )
        .await;
    let key = format!(
        "{PER_USER_TOKEN_KEY_PREFIX}{}:{}",
        input.user_id, input.server_id
    );
    let stored = cache.backend.entry(&key);
    let token = cache
        .get_token(&input.user_id, &input.server_id)
        .await
        .map(|token| token.access_token);
    Ok(json!({
        "token": token,
        "raw_differs": stored.is_some_and(|raw| raw != input.access_token),
        "ttl": cache.backend.recorded_ttl(&key),
    }))
}

pub(super) async fn per_user_delete(input: &PerUserDeleteInput) -> Result<Value, Box<HelperError>> {
    let cache = PerUserCache {
        backend: FakeBackend::new(false, false, input.fail_delete),
        codec: support::codec(),
    };
    cache
        .set(&input.user_id, &input.server_id, "token-value", 300, None)
        .await;
    cache.delete(&input.user_id, &input.server_id).await;
    let token = cache
        .get_token(&input.user_id, &input.server_id)
        .await
        .map(|token| token.access_token);
    Ok(json!({"token_after": token}))
}

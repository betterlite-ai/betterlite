use std::collections::BTreeSet;
use std::iter::repeat_with;
use std::sync::Arc;

use futures_util::future::try_join_all;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::mcp::oauth_caches::per_user::PerUserToken;
use betterlite_server::mcp::oauth_caches::{MCPOAuth2TokenCache, MCPPerUserTokenCache};
use betterlite_server::mcp::outbound_credentials::PER_USER_TOKEN_KEY_PREFIX;
use betterlite_server::mcp::outbound_credentials::client_credentials::token_post::M2mTokenEndpointPost;
use betterlite_server::mcp::outbound_credentials::dual_cache::DualCacheTokenCacheBackend;

use super::support::{
    self, Failure, FakeBackend, FixedClock, OutcomeSpec, RecordedCall, ScriptedPost, ServerSpec,
    ServersInput, authorization,
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
pub(super) struct PerUserInput {
    user_id: String,
    server_id: String,
    #[serde(default)]
    seed: Option<String>,
    #[serde(default)]
    garbage: bool,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    ttl: i64,
    #[serde(default)]
    identity_binding_proof: Option<String>,
    #[serde(default)]
    fail: Option<Failure>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PerUserTtlInput {
    pub(super) server: ServerSpec,
    pub(super) expires_in: Option<i64>,
}

type PerUserCache = MCPPerUserTokenCache<DualCacheTokenCacheBackend<FakeBackend>>;

struct OAuth2Call {
    server: MCPServer,
    scripted: Arc<ScriptedPost>,
    cache: MCPOAuth2TokenCache,
}

impl OAuth2Call {
    fn new(spec: &ServerSpec, outcomes: &[OutcomeSpec]) -> Result<Self, Box<HelperError>> {
        let scripted = Arc::new(ScriptedPost::new(outcomes)?);
        let poster: Arc<dyn M2mTokenEndpointPost> = Arc::<ScriptedPost>::clone(&scripted);
        let server: MCPServer = serde_json::from_value(spec.0.clone())
            .map_err(|error| support::value_error(error.to_string()))?;
        Ok(Self {
            server,
            scripted,
            cache: MCPOAuth2TokenCache::new(poster, Arc::new(FixedClock))
                .map_err(|error| support::value_error(error.to_string()))?,
        })
    }

    async fn cached(&self) -> Result<Option<String>, Box<HelperError>> {
        self.cache
            .async_get_token(&self.server)
            .await
            .map_err(|error| support::value_error(error.to_string()))
    }

    async fn fetched(&self) -> Result<(String, i64), Box<HelperError>> {
        self.cache
            .fetch_token(&self.server)
            .await
            .map_err(|error| support::value_error(error.to_string()))
    }
}

struct PerUserCall {
    cache: PerUserCache,
    user_id: String,
    server_id: String,
}

impl PerUserCall {
    fn new(input: &PerUserInput) -> Self {
        Self {
            cache: per_user_cache(input),
            user_id: input.user_id.clone(),
            server_id: input.server_id.clone(),
        }
    }

    fn key(&self) -> String {
        let prefix = PER_USER_TOKEN_KEY_PREFIX;
        format!("{prefix}{}:{}", self.user_id, self.server_id)
    }

    async fn seed(&self, token: &str, ttl: i64) {
        self.cache
            .set(
                &self.user_id,
                &self.server_id,
                PerUserToken {
                    access_token: token,
                    ttl_seconds: ttl,
                    identity_binding_proof: None,
                },
            )
            .await;
    }

    async fn stored(&self) -> Option<String> {
        self.cache
            .get_token(&self.user_id, &self.server_id)
            .await
            .map(|token| token.access_token)
    }
}

fn per_user_cache(input: &PerUserInput) -> PerUserCache {
    let failures: BTreeSet<Failure> = input.fail.into_iter().collect();
    PerUserCache {
        backend: DualCacheTokenCacheBackend {
            cache: FakeBackend::new(failures),
            codec: support::codec(),
            key_prefix: PER_USER_TOKEN_KEY_PREFIX,
        },
    }
}

fn server_identity(spec: &ServerSpec) -> Result<String, Box<HelperError>> {
    let server: MCPServer = serde_json::from_value(spec.0.clone())
        .map_err(|error| support::value_error(error.to_string()))?;
    Ok(MCPOAuth2TokenCache::token_identity(&server))
}

fn set_view(token: Option<String>, raw_differs: bool, ttl: Option<i64>) -> Value {
    json!({
        "token": token,
        "raw_differs": raw_differs,
        "ttl": ttl,
    })
}

fn fetch_view(
    token: String,
    ttl: i64,
    call: Option<RecordedCall>,
) -> Result<Value, Box<HelperError>> {
    let recorded =
        call.ok_or_else(|| support::value_error("the token request was not posted".to_owned()))?;
    let mut form = recorded.form;
    form.sort();
    Ok(json!({
        "token": token,
        "ttl": ttl,
        "form": form,
        "authorization": authorization(&recorded.headers),
    }))
}

async fn concurrent_tokens(
    call: &OAuth2Call,
    calls: usize,
    concurrent: bool,
) -> Result<Vec<Option<String>>, Box<HelperError>> {
    let futures = repeat_with(|| call.cached()).take(calls);
    if concurrent {
        return try_join_all(futures).await;
    }
    let mut sequential = Vec::with_capacity(calls);
    for future in futures {
        sequential.push(future.await?);
    }
    Ok(sequential)
}

pub(super) fn identity(input: &ServersInput) -> Result<Value, Box<HelperError>> {
    let identities = input
        .servers
        .iter()
        .map(server_identity)
        .collect::<Result<Vec<String>, Box<HelperError>>>()?;
    Ok(json!({"identities": identities}))
}

pub(super) async fn get_token(input: &GetTokenInput) -> Result<Value, Box<HelperError>> {
    let call = OAuth2Call::new(&input.server, &input.outcomes)?;
    let tokens = concurrent_tokens(&call, input.calls, input.concurrent).await?;
    Ok(json!({"tokens": tokens, "posts": call.scripted.post_count()}))
}

pub(super) async fn fetch(input: &FetchInput) -> Result<Value, Box<HelperError>> {
    let call = OAuth2Call::new(&input.server, &input.outcomes)?;
    let (token, ttl) = call.fetched().await?;
    fetch_view(token, ttl, call.scripted.first_call())
}

pub(super) async fn invalidate(input: &InvalidateInput) -> Result<Value, Box<HelperError>> {
    let call = OAuth2Call::new(&input.server, &input.outcomes)?;
    let first = call.cached().await?;
    call.cache.invalidate(&call.server.fields.server_id);
    let second = call.cached().await?;
    Ok(json!({"tokens": vec![first, second], "posts": call.scripted.post_count()}))
}

pub(super) async fn per_user_get(input: &PerUserInput) -> Result<Value, Box<HelperError>> {
    let call = PerUserCall::new(input);
    if let Some(seed) = &input.seed {
        call.seed(seed, 300).await;
    }
    if input.garbage {
        call.cache.backend.cache.seed(&call.key(), "garbage");
    }
    Ok(json!({"token": call.stored().await}))
}

pub(super) async fn per_user_set(input: &PerUserInput) -> Result<Value, Box<HelperError>> {
    let call = PerUserCall::new(input);
    call.seed(&input.access_token, input.ttl).await;
    let stored = call.cache.backend.cache.entry(&call.key());
    let differs = stored.is_some_and(|raw| raw != input.access_token);
    let ttl = call.cache.backend.cache.recorded_ttl(&call.key());
    Ok(set_view(call.stored().await, differs, ttl))
}

pub(super) async fn per_user_delete(input: &PerUserInput) -> Result<Value, Box<HelperError>> {
    let call = PerUserCall::new(input);
    let seed = (support::codec().encrypt)("token-value");
    call.cache.backend.cache.seed(&call.key(), &seed);
    call.cache.delete(&call.user_id, &call.server_id).await;
    Ok(json!({"token_after": call.stored().await}))
}

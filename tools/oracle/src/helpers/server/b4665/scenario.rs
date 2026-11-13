use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::auth_config::AuthConfig;
use betterlite_mcp::credential_contracts::auth_config::oauth::TokenExchangeConfig;
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use betterlite_mcp::credential_contracts::server_spec::ServerSpec;
use betterlite_server::mcp::outbound_credentials::refresh_store::InProcessRefreshCoordinator;
use betterlite_server::mcp::outbound_credentials::token_exchanger::{
    DEFAULT_TTL, EXPIRY_BUFFER, ExchangeBody, ExchangeHttpPost, ExchangeRejection, MIN_TTL,
    OboTokenExchanger,
};
use betterlite_server::mcp::outbound_credentials::{
    InMemoryTokenCacheBackend, OAuthToken, TokenCacheBackend, TokenCacheEntries, TokenKey,
    WallClock,
};
use chrono::{DateTime, Utc};
use futures_util::future::join_all;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tokio::task::yield_now;

use super::input::{ConfigSpec, Input, ResponseKind, ResponseSpec, StepOp, StepSpec, token_config};
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

const DEFAULT_SERVER_ID: &str = "srv";
const DEFAULT_CONCURRENT_CALLERS: usize = 2;
const DEFAULT_CACHE_MAX_SIZE: usize = 4096;

struct ManualClock(Mutex<DateTime<Utc>>);

impl ManualClock {
    fn set(&self, now_ms: i64) -> Result<(), Box<HelperError>> {
        let instant = DateTime::from_timestamp_millis(now_ms)
            .ok_or_else(|| std::io::Error::other("now_ms is outside the supported range"))?;
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = instant;
        Ok(())
    }
}

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct RecordingCache {
    inner: InMemoryTokenCacheBackend,
    events: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl TokenCacheBackend for RecordingCache {
    async fn get(&self, key: &TokenKey) -> Option<OAuthToken> {
        let token = self.inner.get(key).await;
        let hit = token.is_some();
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "op": "get",
                "key": key.user_id,
                "server": key.server_id,
                "hit": hit,
            }));
        token
    }

    async fn set(&self, key: &TokenKey, token: &OAuthToken, ttl: Duration) {
        let ttl_ms = i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX);
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "op": "set",
                "key": key.user_id,
                "server": key.server_id,
                "ttl_ms": ttl_ms,
            }));
        self.inner.set(key, token, ttl).await;
    }

    async fn delete(&self, key: &TokenKey) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "op": "delete",
                "key": key.user_id,
                "server": key.server_id,
            }));
        self.inner.delete(key).await;
    }
}

struct PostState {
    responses: VecDeque<ResponseSpec>,
    calls: Vec<Value>,
}

struct ScriptedPost {
    state: Mutex<PostState>,
}

fn expose(form: &BTreeMap<String, SecretString>) -> BTreeMap<String, String> {
    form.iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

#[async_trait]
impl ExchangeHttpPost for ScriptedPost {
    async fn post(
        &self,
        url: &str,
        form: &BTreeMap<String, SecretString>,
        headers: &BTreeMap<String, SecretString>,
    ) -> Result<Option<ExchangeBody>, ExchangeRejection> {
        let recorded = json!({
            "url": url,
            "form": expose(form),
            "headers": expose(headers),
        });
        let scripted = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.calls.push(recorded);
            state.responses.pop_front()
        };
        yield_now().await;
        let Some(outcome) = scripted else {
            return Ok(None);
        };
        match outcome.kind {
            ResponseKind::Body => Ok(Some(outcome.body.unwrap_or_default())),
            ResponseKind::Transport => Ok(None),
            ResponseKind::SubjectRejected => Err(ExchangeRejection::SubjectRejected {
                detail: outcome.detail.unwrap_or_default(),
                claims: outcome.claims,
            }),
            ResponseKind::ClientFault => Err(ExchangeRejection::ClientFault {
                code: "invalid_client".to_owned(),
            }),
        }
    }
}

type ScenarioExchanger = OboTokenExchanger<
    ScriptedPost,
    RecordingCache,
    InProcessRefreshCoordinator<Option<OAuthToken>, ExchangeRejection>,
>;

const fn ttl_of(milliseconds: Option<u64>, default: Duration) -> Duration {
    let Some(value) = milliseconds else {
        return default;
    };
    Duration::from_millis(value)
}

fn token_view(token: &OAuthToken) -> Value {
    json!({
        "access_token": token.access_token,
        "expires_at_ms": token.expires_at.map(|instant| instant.timestamp_millis()),
        "refresh_token": token.refresh_token,
        "scopes": token.scopes,
        "identity_binding_proof": token.identity_binding_proof,
    })
}

fn error_view(error: &CredError) -> Value {
    let tag = match error {
        CredError::Unauthorized(_) => "unauthorized",
        CredError::Misconfigured(_) => "misconfigured",
        CredError::UrlCredentialsNotAllowed => "url_credentials_not_allowed",
        CredError::UpstreamUnavailable(_) => "upstream_unavailable",
        CredError::UnsupportedMode(_) => "unsupported_mode",
        CredError::PreconditionRequired(_) => "precondition_required",
        CredError::NotImplemented(_) => "not_implemented",
    };
    json!({"kind": "error", "tag": tag, "claims": error_claims(error)})
}

fn error_claims(error: &CredError) -> Option<String> {
    match error {
        CredError::Unauthorized(unauthorized) => unauthorized.claims.clone(),
        CredError::Misconfigured(_)
        | CredError::UrlCredentialsNotAllowed
        | CredError::UpstreamUnavailable(_)
        | CredError::UnsupportedMode(_)
        | CredError::PreconditionRequired(_)
        | CredError::NotImplemented(_) => None,
    }
}

fn result_view(result: CredResult<OAuthToken>) -> Value {
    match result {
        Ok(token) => json!({"kind": "ok", "token": token_view(&token)}),
        Err(error) => error_view(&error),
    }
}

fn step_server(config: &ConfigSpec) -> ServerSpec {
    ServerSpec {
        server_id: DEFAULT_SERVER_ID.to_owned(),
        resource: String::new(),
        config: AuthConfig::TokenExchange(token_config(config)),
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    runtime.block_on(scenario(input))
}

async fn scenario(input: &Input) -> HelperResult {
    let clock = Arc::new(ManualClock(Mutex::new(DateTime::<Utc>::UNIX_EPOCH)));
    let run_clock: Arc<ManualClock> = Arc::clone(&clock);
    let manual_clock: Arc<ManualClock> = Arc::clone(&clock);
    let cache_clock: Arc<dyn WallClock> = manual_clock;
    let events = Arc::new(Mutex::new(Vec::new()));
    let cache = RecordingCache {
        inner: InMemoryTokenCacheBackend {
            max_size: DEFAULT_CACHE_MAX_SIZE,
            clock: cache_clock,
            entries: TokenCacheEntries::default(),
        },
        events: Arc::clone(&events),
    };
    let exchanger = OboTokenExchanger {
        post: ScriptedPost {
            state: Mutex::new(PostState {
                responses: VecDeque::new(),
                calls: Vec::new(),
            }),
        },
        cache,
        coordinator: InProcessRefreshCoordinator::default(),
        clock,
        default_ttl: ttl_of(input.default_ttl_ms, DEFAULT_TTL),
        min_ttl: ttl_of(input.min_ttl_ms, MIN_TTL),
        expiry_buffer: ttl_of(input.expiry_buffer_ms, EXPIRY_BUFFER),
    };
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        steps.push(run_step(&exchanger, &run_clock, &events, step, input).await?);
    }
    Ok(json!({"steps": steps}))
}

fn take_events(events: &Arc<Mutex<Vec<Value>>>) -> Vec<Value> {
    let mut guard = events.lock().unwrap_or_else(PoisonError::into_inner);
    let taken = std::mem::take(&mut *guard);
    drop(guard);
    taken
}

fn prepare_step(exchanger: &ScenarioExchanger, events: &Arc<Mutex<Vec<Value>>>, step: &StepSpec) {
    let mut state = exchanger
        .post
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    state.responses = step.responses.iter().cloned().collect();
    state.calls.clear();
    drop(state);
    drop(take_events(events));
}

async fn run_concurrent(
    exchanger: &ScenarioExchanger,
    step: &StepSpec,
    server: &ServerSpec,
    config: &TokenExchangeConfig,
    tenant: &str,
) -> Value {
    let count = step.count.unwrap_or(DEFAULT_CONCURRENT_CALLERS);
    let mut pending = Vec::with_capacity(count);
    for _ in 0..count {
        pending.push(exchanger.exchange(&step.subject_token, server, config, tenant));
    }
    let results = join_all(pending).await;
    let posts = exchanger
        .post
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .calls
        .len();
    json!({
        "op": "concurrent",
        "results": results.into_iter().map(result_view).collect::<Vec<Value>>(),
        "posts": posts,
    })
}

async fn run_step(
    exchanger: &ScenarioExchanger,
    clock: &ManualClock,
    events: &Arc<Mutex<Vec<Value>>>,
    step: &StepSpec,
    input: &Input,
) -> HelperResult {
    if let Some(now_ms) = step.now_ms {
        clock.set(now_ms)?;
    }
    let config_spec = step.config.as_ref().unwrap_or(&input.config);
    let server = step_server(config_spec);
    let config = token_config(config_spec);
    let tenant = step.tenant_id.clone().unwrap_or_default();
    prepare_step(exchanger, events, step);
    if step.op == StepOp::Concurrent {
        return Ok(run_concurrent(exchanger, step, &server, &config, &tenant).await);
    }
    let result = if step.op == StepOp::Exchange {
        result_view(
            exchanger
                .exchange(&step.subject_token, &server, &config, &tenant)
                .await,
        )
    } else {
        exchanger
            .invalidate(&step.subject_token, &server, &config, &tenant)
            .await;
        json!({"kind": "none"})
    };
    let calls = exchanger
        .post
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .calls
        .clone();
    Ok(json!({
        "op": step.op.as_str(),
        "result": result,
        "events": take_events(events),
        "calls": calls,
    }))
}

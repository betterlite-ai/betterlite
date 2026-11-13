use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_mcp::credential_contracts::auth_config::AuthConfig;
use betterlite_mcp::credential_contracts::auth_config::oauth::TokenExchangeConfig;
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use betterlite_mcp::credential_contracts::server_spec::ServerSpec;
use betterlite_server::mcp::outbound_credentials::refresh_store::InProcessRefreshCoordinator;
use betterlite_server::mcp::outbound_credentials::token_exchanger::{
    DEFAULT_TTL, EXPIRY_BUFFER, ExchangeRejection, MIN_TTL, OboTokenExchanger,
};
use betterlite_server::mcp::outbound_credentials::{
    InMemoryTokenCacheBackend, OAuthToken, TokenCacheEntries, WallClock,
};
use chrono::{DateTime, Utc};
use futures_util::future::join_all;
use serde_json::{Value, json};

use super::fakes::{ManualClock, PostState, RecordingCache, ScriptedPost};
use super::input::{ConfigSpec, Input, StepOp, StepSpec, token_config};
use crate::helpers::{HelperResult, OrHelperError as _};

const DEFAULT_SERVER_ID: &str = "srv";
const DEFAULT_CONCURRENT_CALLERS: usize = 2;
const DEFAULT_CACHE_MAX_SIZE: usize = 4096;

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

fn ok_view(token: &OAuthToken) -> Value {
    json!({
        "kind": "ok",
        "token": token_view(token),
    })
}

fn refused_view(error: &CredError) -> Value {
    let (tag, claims) = match error {
        CredError::Unauthorized(unauthorized) => ("unauthorized", unauthorized.claims.clone()),
        CredError::Misconfigured(_) => ("misconfigured", None),
        CredError::UrlCredentialsNotAllowed => ("url_credentials_not_allowed", None),
        CredError::UpstreamUnavailable(_) => ("upstream_unavailable", None),
        CredError::UnsupportedMode(_) => ("unsupported_mode", None),
        CredError::PreconditionRequired(_) => ("precondition_required", None),
        CredError::NotImplemented(_) => ("not_implemented", None),
    };
    json!({
        "kind": "error",
        "tag": tag,
        "claims": claims,
    })
}

fn result_view(result: CredResult<OAuthToken>) -> Value {
    match result {
        Ok(token) => ok_view(&token),
        Err(error) => refused_view(&error),
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

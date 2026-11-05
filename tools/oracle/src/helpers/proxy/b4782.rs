use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::response::{IntoResponse as _, Response};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_proxy::agent_endpoints::databricks_oauth::config::{
    AgentLitellmParams, parse_databricks_oauth_config,
};
use betterlite_proxy::agent_endpoints::databricks_oauth::header::resolve_oauth_header;
use betterlite_proxy::agent_endpoints::databricks_oauth::token_cache::DatabricksAppOAuthTokenCache;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

// The Python side swaps the same placeholder for its own mock address.
const PLACEHOLDER: &str = "UPSTREAM";
const MAX_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Deserialize)]
pub(crate) struct Upstream {
    status: u16,
    body: String,
    #[serde(default)]
    delay_ms: u64,
}

#[derive(Deserialize)]
pub(crate) struct Round {
    params: Value,
    #[serde(default)]
    advance: u32,
    callers: Option<usize>,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Parse {
        params: AgentLitellmParams,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Header {
        upstream: Upstream,
        rounds: Vec<Round>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
}

#[derive(Serialize)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    content_type: Option<String>,
    body: String,
}

struct Mock {
    upstream: Upstream,
    seen: Mutex<Vec<Seen>>,
}

fn failure<E>(class: &str, error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

async fn serve(State(mock): State<Arc<Mock>>, request: Request) -> Response {
    let (head, body) = request.into_parts();
    let text = to_bytes(body, MAX_REQUEST_BYTES).await.unwrap_or_default();
    let header = |name| {
        head.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    mock.seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Seen {
            method: head.method.to_string(),
            path: head.uri.path().to_owned(),
            authorization: header(AUTHORIZATION),
            content_type: header(CONTENT_TYPE),
            body: String::from_utf8_lossy(&text).into_owned(),
        });
    sleep(Duration::from_millis(mock.upstream.delay_ms)).await;
    let status =
        StatusCode::from_u16(mock.upstream.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (
        status,
        [(CONTENT_TYPE, "application/json")],
        Body::from(mock.upstream.body.clone()),
    )
        .into_response()
}

fn parse(params: &AgentLitellmParams, env: &BTreeMap<String, String>) -> HelperResult {
    let parsed =
        parse_databricks_oauth_config(params, env).map_err(|error| failure("ValueError", error))?;
    Ok(parsed.map_or(Value::Null, |config| {
        json!({
            "cache_key": config.cache_key(),
            "client_id": config.client_id,
            "client_secret": config.client_secret.expose_secret(),
            "scope": config.scope,
            "token_url": config.token_url,
        })
    }))
}

struct Harness {
    cache: Arc<DatabricksAppOAuthTokenCache>,
    now: Arc<AtomicU32>,
    env: Arc<BTreeMap<String, String>>,
    url: String,
}

fn harness(env: &BTreeMap<String, String>, url: String) -> Result<Harness, Box<HelperError>> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .map_err(|error| failure("ProxyError", error))?;
    let client =
        ProxiedClient::outbound(&Arc::new(policy)).map_err(|error| failure("ProxyError", error))?;
    let now = Arc::new(AtomicU32::new(0));
    let reader = Arc::clone(&now);
    let clock = Arc::new(move || f64::from(reader.load(Ordering::SeqCst)));
    let cache = DatabricksAppOAuthTokenCache::new(client, clock)
        .map_err(|error| failure("ValueError", error))?;
    Ok(Harness {
        cache: Arc::new(cache),
        now,
        env: Arc::new(env.clone()),
        url,
    })
}

async fn run_round(
    harness: &Harness,
    round: &Round,
) -> Result<Vec<Option<String>>, Box<HelperError>> {
    harness.now.fetch_add(round.advance, Ordering::SeqCst);
    let text = round.params.to_string().replace(PLACEHOLDER, &harness.url);
    let params = Arc::new(
        serde_json::from_str::<AgentLitellmParams>(&text)
            .map_err(|error| failure("InvalidInput", error))?,
    );
    let mut calls = JoinSet::new();
    for _ in 0..round.callers.unwrap_or(1) {
        let cache = Arc::clone(&harness.cache);
        let call_params = Arc::clone(&params);
        let environment = Arc::clone(&harness.env);
        calls.spawn(async move {
            resolve_oauth_header(&cache, &call_params, environment.as_ref()).await
        });
    }
    let mut answers = Vec::new();
    while let Some(joined) = calls.join_next().await {
        let resolved = joined
            .map_err(|error| failure("JoinError", error))?
            .map_err(|error| failure("ValueError", error))?;
        answers.push(resolved.map(|header| header.expose_secret().to_owned()));
    }
    Ok(answers)
}

async fn drive(
    upstream: &Upstream,
    rounds: &[Round],
    env: &BTreeMap<String, String>,
) -> HelperResult {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| failure("IoError", error))?;
    let address = listener
        .local_addr()
        .map_err(|error| failure("IoError", error))?;
    let mock = Arc::new(Mock {
        upstream: upstream.clone(),
        seen: Mutex::new(Vec::new()),
    });
    let router = Router::new().fallback(serve).with_state(Arc::clone(&mock));
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let rig = harness(env, format!("http://{address}"))?;
    let mut headers = Vec::new();
    for round in rounds {
        headers.push(run_round(&rig, round).await?);
    }
    let locks = rig.cache.live_lock_count().await;
    server.abort();
    let seen = mock.seen.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(json!({"headers": headers, "locks": locks, "requests": *seen}))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Parse { params, env } => parse(params, env),
            Input::Header {
                upstream,
                rounds,
                env,
            } => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| failure("RuntimeError", error))?;
                runtime.block_on(drive(upstream, rounds, env))
            }
        }
    }
}

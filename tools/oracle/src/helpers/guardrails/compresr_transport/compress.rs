use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use betterlite_guardrails::compresr_transport::{
    CompresrTransport, CompressOutcome, CompressTarget, CompressionResult,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::task::JoinSet;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::Settings;
use super::logs::Logs;
use crate::helpers::{HelperResult, OrHelperError as _};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const API_KEY: &str = "test-key";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Goal {
    context: String,
    query: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    status: u16,
    body: String,
}

#[derive(Default)]
struct Seen {
    paths: Vec<Value>,
    bodies: Vec<Value>,
    headers: Vec<Value>,
}

#[derive(Clone)]
struct Mock {
    status: StatusCode,
    body: String,
    seen: Arc<Mutex<Seen>>,
}

async fn record(State(mock): State<Mock>, request: Request) -> (StatusCode, String) {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_REQUEST_BYTES).await.unwrap_or_default();
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    let mut seen = mock.seen.lock().unwrap_or_else(PoisonError::into_inner);
    seen.paths.push(json!(parts.uri.path()));
    seen.bodies
        .push(serde_json::from_slice(&bytes).unwrap_or(Value::Null));
    seen.headers.push(json!({
        "content_type": header("content-type"),
        "api_key": header("x-api-key"),
    }));
    (mock.status, mock.body)
}

fn result_json(result: &CompressionResult) -> Value {
    json!({
        "compressed_context": result.compressed_context,
        "original_tokens": result.original_tokens,
        "compressed_tokens": result.compressed_tokens,
    })
}

pub(super) fn run(settings: &Settings, goals: &[Goal], reply: &Reply) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mock = Mock {
        status: StatusCode::from_u16(reply.status).or_value_error()?,
        body: reply.body.clone(),
        seen: Arc::clone(&seen),
    };
    let listener = runtime
        .block_on(TcpListener::bind("127.0.0.1:0"))
        .or_value_error()?;
    let address = listener.local_addr().or_value_error()?;
    let mut server = JoinSet::new();
    let router = Router::new().fallback(record).with_state(mock);
    let _guard = runtime.enter();
    server.spawn(async move {
        // The server stops when the helper returns and the set aborts it.
        let _served = axum::serve(listener, router).await;
    });
    let mut transport_settings = settings.transport();
    transport_settings.api_base = format!("http://{address}");
    let transport = CompresrTransport::new(
        transport_settings,
        SecretString::from(API_KEY),
        reqwest::Client::default(),
    )
    .or_value_error()?;
    let targets: Vec<CompressTarget> = goals
        .iter()
        .map(|goal| CompressTarget {
            context: goal.context.clone(),
            query: goal.query.clone(),
        })
        .collect();
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let outcome = with_default(subscriber, || {
        runtime.block_on(transport.compress(&targets))
    });
    let mut answer = match outcome {
        Ok(CompressOutcome::Results(results)) => json!({
            "outcome": "results",
            "results": results.iter().map(result_json).collect::<Vec<_>>(),
        }),
        Ok(CompressOutcome::Uncompressed) => json!({"outcome": "uncompressed"}),
        Err(error) => json!({
            "outcome": "raised",
            "status": error.status_code(),
            "detail": {"error": error.to_string()},
        }),
    };
    let seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
    answer["path"] = Value::Array(seen.paths.clone());
    answer["body"] = Value::Array(seen.bodies.clone());
    answer["headers"] = Value::Array(seen.headers.clone());
    answer["logs"] = logs.records();
    Ok(answer)
}

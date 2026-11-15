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
use serde_json::{Map, Value, json};
use tokio::net::TcpListener;
use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinSet;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::Settings;
use super::logs::Logs;
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

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
    drop(seen);
    (mock.status, mock.body)
}

fn result_json(result: &CompressionResult) -> Value {
    json!({
        "compressed_context": result.compressed_context,
        "original_tokens": result.original_tokens,
        "compressed_tokens": result.compressed_tokens,
    })
}

fn serve_mock(
    runtime: &Runtime,
    reply: &Reply,
    seen: &Arc<Mutex<Seen>>,
) -> Result<(String, JoinSet<()>), Box<HelperError>> {
    let mock = Mock {
        status: StatusCode::from_u16(reply.status).or_value_error()?,
        body: reply.body.clone(),
        seen: Arc::clone(seen),
    };
    let listener = runtime
        .block_on(TcpListener::bind("127.0.0.1:0"))
        .or_value_error()?;
    let address = listener.local_addr().or_value_error()?;
    let router = Router::new().fallback(record).with_state(mock);
    let mut server = JoinSet::new();
    let _guard = runtime.enter();
    server.spawn(async move {
        // The server stops when the helper returns and the set aborts it.
        let _served = axum::serve(listener, router).await;
    });
    Ok((format!("http://{address}"), server))
}

fn transport_for(
    settings: &Settings,
    api_base: &str,
) -> Result<CompresrTransport, Box<HelperError>> {
    let mut transport_settings = settings.transport()?;
    api_base.clone_into(&mut transport_settings.api_base);
    CompresrTransport::new(
        transport_settings,
        SecretString::from(API_KEY),
        reqwest::Client::default(),
    )
    .or_value_error()
}

fn targets_for(goals: &[Goal]) -> Vec<CompressTarget> {
    goals
        .iter()
        .map(|goal| CompressTarget {
            context: goal.context.clone(),
            query: goal.query.clone(),
        })
        .collect()
}

pub(super) fn run(settings: &Settings, goals: &[Goal], reply: &Reply) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (api_base, _server) = serve_mock(&runtime, reply, &seen)?;
    let transport = transport_for(settings, &api_base)?;
    let targets = targets_for(goals);
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let outcome = with_default(subscriber, || {
        runtime.block_on(transport.compress(&targets))
    });
    let mut answer = Map::new();
    match outcome {
        Ok(CompressOutcome::Results(results)) => {
            answer.insert("outcome".to_owned(), json!("results"));
            answer.insert(
                "results".to_owned(),
                json!(results.iter().map(result_json).collect::<Vec<_>>()),
            );
        }
        Ok(CompressOutcome::Uncompressed) => {
            answer.insert("outcome".to_owned(), json!("uncompressed"));
        }
        Err(error) => {
            answer.insert("outcome".to_owned(), json!("raised"));
            answer.insert("status".to_owned(), json!(error.status_code()));
            answer.insert("detail".to_owned(), json!({"error": error.to_string()}));
        }
    }
    let recorded = seen.lock().unwrap_or_else(PoisonError::into_inner);
    answer.insert("path".to_owned(), Value::Array(recorded.paths.clone()));
    answer.insert("body".to_owned(), Value::Array(recorded.bodies.clone()));
    answer.insert("headers".to_owned(), Value::Array(recorded.headers.clone()));
    drop(recorded);
    answer.insert("logs".to_owned(), logs.records());
    Ok(Value::Object(answer))
}

use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_guardrails::alice::EVALUATE_PATH;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::helpers::HelperError;

use super::{AliceApplyArgs, FailureKind, ScriptedAnswer};

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct Recorded {
    path: String,
    af_api_key: Option<String>,
    content_type: Option<String>,
    body: Value,
}

type Recorder = Arc<Mutex<Vec<Recorded>>>;

pub(super) enum Endpoint {
    Serving {
        base: String,
        recorder: Recorder,
        _tasks: JoinSet<io::Result<()>>,
    },
    Silent {
        base: String,
        _listener: TcpListener,
    },
    Refused {
        base: String,
    },
}

impl Endpoint {
    pub(super) fn base(&self) -> &str {
        match self {
            Self::Serving { base, .. } | Self::Silent { base, .. } | Self::Refused { base } => {
                base.as_str()
            }
        }
    }

    pub(super) fn recorded(&self) -> Vec<Recorded> {
        match self {
            Self::Serving { recorder, .. } => match recorder.lock() {
                Ok(entries) => entries.clone(),
                Err(_poisoned) => Vec::new(),
            },
            Self::Silent { .. } | Self::Refused { .. } => Vec::new(),
        }
    }
}

pub(super) async fn start_endpoint(args: &AliceApplyArgs) -> Result<Endpoint, Box<HelperError>> {
    if let Some(answer) = &args.answer {
        return serve(answer.clone()).await;
    }
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let base = format!("http://{address}");
    if args.failure == Some(FailureKind::Timeout) {
        return Ok(Endpoint::Silent {
            base,
            _listener: listener,
        });
    }
    drop(listener);
    Ok(Endpoint::Refused { base })
}

async fn serve(answer: ScriptedAnswer) -> Result<Endpoint, Box<HelperError>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let recorder: Recorder = Arc::new(Mutex::new(Vec::new()));
    let handler_recorder = Arc::clone(&recorder);
    let handler = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let entry_recorder = Arc::clone(&handler_recorder);
        let scripted = answer.clone();
        async move {
            let entry = Recorded {
                path: uri.path().to_owned(),
                af_api_key: header_text(&headers, "af-api-key"),
                content_type: header_text(&headers, "content-type"),
                body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            };
            match entry_recorder.lock() {
                Ok(mut entries) => entries.push(entry),
                Err(_poisoned) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "recorder poisoned")
                        .into_response();
                }
            }
            answer_response(&scripted)
        }
    };
    let router = Router::new().route(EVALUATE_PATH, post(handler));
    let mut tasks = JoinSet::new();
    tasks.spawn(axum::serve(listener, router).into_future());
    Ok(Endpoint::Serving {
        base,
        recorder,
        _tasks: tasks,
    })
}

fn answer_response(answer: &ScriptedAnswer) -> Response {
    let status = match StatusCode::from_u16(answer.status) {
        Ok(status) => status,
        Err(_error) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if let Some(raw) = &answer.raw {
        return (status, raw.clone()).into_response();
    }
    match serde_json::to_vec(&answer.body) {
        Ok(body) => (status, [(header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(_error) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub(super) fn build_client() -> Result<reqwest::Client, Box<HelperError>> {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: "RequestError".to_owned(),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: source.to_string(),
            })
        })
}

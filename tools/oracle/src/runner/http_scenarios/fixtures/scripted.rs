use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io::Error;
use std::net::TcpListener;
use std::str;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{Response as HttpResponse, StatusCode, request::Parts};
use futures_util::future::{Either, select};
use serde_json::Value;
use tokio::sync::oneshot;
use tokio::{net, runtime};

use crate::OracleError;
use crate::case::UpstreamAnswer;
use crate::http::MAX_BODY;
use crate::mock::ProviderRequest;
use crate::multipart_oracle::request::answer_body;
use crate::routing_mock_scripts::redact;

struct ProviderState {
    answers: Vec<UpstreamAnswer>,
    index: usize,
    requests: Vec<ProviderRequest>,
}

pub(crate) struct ScriptedProvider {
    pub(crate) base_url: String,
    state: Arc<Mutex<ProviderState>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<Result<(), Error>>>,
}

impl ScriptedProvider {
    pub(crate) fn start(answers: Vec<UpstreamAnswer>) -> Result<Self, OracleError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(OracleError::MockIo)?;
        listener
            .set_nonblocking(true)
            .map_err(OracleError::MockIo)?;
        let address = listener.local_addr().map_err(OracleError::MockIo)?;
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(OracleError::MockIo)?;
        let state = Arc::new(Mutex::new(ProviderState {
            answers,
            index: 0,
            requests: Vec::new(),
        }));
        let router = Router::new()
            .fallback(handle)
            .with_state(Arc::clone(&state));
        let (stop, stopped) = oneshot::channel();
        let thread = thread::Builder::new()
            .spawn(move || runtime.block_on(serve(listener, router, stopped)))
            .map_err(OracleError::MockIo)?;
        Ok(Self {
            base_url: format!("http://{address}"),
            state,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    pub(crate) fn requests(&self) -> Result<Vec<ProviderRequest>, OracleError> {
        let state = self
            .state
            .lock()
            .map_err(|error| OracleError::MockIo(Error::other(error.to_string())))?;
        Ok(state.requests.clone())
    }

    pub(crate) fn reset(&self) -> Result<(), OracleError> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| OracleError::MockIo(Error::other(error.to_string())))?;
        state.index = 0;
        state.requests.clear();
        drop(state);
        Ok(())
    }
}

impl Drop for ScriptedProvider {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _sent = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

async fn serve(
    listener: TcpListener,
    router: Router,
    stopped: oneshot::Receiver<()>,
) -> Result<(), Error> {
    let socket = net::TcpListener::from_std(listener)?;
    let closing = async move {
        let _stopped = stopped.await;
    };
    let served = Box::pin(axum::serve(socket, router).into_future());
    let stopping = Box::pin(closing);
    match select(served, stopping).await {
        Either::Left((finished, _)) => finished,
        Either::Right(((), _)) => Ok(()),
    }
}

async fn handle(
    State(state): State<Arc<Mutex<ProviderState>>>,
    request: Request,
) -> HttpResponse<Body> {
    let (parts, body) = request.into_parts();
    if parts.method == "GET" && parts.uri.path() == "/v1/models" {
        return HttpResponse::new(Body::from(r#"{"object":"list","data":[]}"#));
    }
    let bytes = match to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(_source) => return failure(StatusCode::PAYLOAD_TOO_LARGE),
    };
    let Ok(captured) = capture(&parts, &bytes) else {
        return failure(StatusCode::BAD_REQUEST);
    };
    let queued = {
        let Ok(mut recorded) = state.lock() else {
            return failure(StatusCode::INTERNAL_SERVER_ERROR);
        };
        recorded.requests.push(captured);
        let answer = recorded.answers.get(recorded.index).cloned();
        recorded.index = recorded.index.saturating_add(1);
        answer
    };
    queued.map_or_else(|| failure(StatusCode::INTERNAL_SERVER_ERROR), reply)
}

fn capture(parts: &Parts, bytes: &[u8]) -> Result<ProviderRequest, Error> {
    let mut headers = parts
        .headers
        .iter()
        .map(|(name, value)| {
            Ok((
                name.to_string(),
                str::from_utf8(value.as_bytes())?.to_owned(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, str::Utf8Error>>()
        .map_err(Error::other)?;
    redact(&mut headers);
    let raw = headers.get("content-type").is_some_and(|value| {
        value.starts_with("multipart/form-data") || value.starts_with("application/octet-stream")
    });
    let body = if raw {
        Value::Array(bytes.iter().map(|byte| Value::from(*byte)).collect())
    } else {
        let text = str::from_utf8(bytes).map_err(Error::other)?;
        serde_json::from_str(text).unwrap_or_else(|_source| Value::String(text.to_owned()))
    };
    Ok(ProviderRequest {
        method: parts.method.to_string(),
        path: parts.uri.to_string(),
        headers,
        body,
    })
}

fn reply(mut answer: UpstreamAnswer) -> HttpResponse<Body> {
    let body = if let Some((content_type, bytes)) = answer_body(&answer).ok().flatten() {
        answer
            .headers
            .insert("content-type".to_owned(), content_type);
        bytes
    } else if let Some(json) = answer.body {
        serde_json::to_vec(&json).unwrap_or_default()
    } else {
        let mut bytes = Vec::new();
        for event in answer.events.unwrap_or_default() {
            bytes.extend_from_slice(format!("data: {event}\n").as_bytes());
        }
        bytes
    };
    let mut response = HttpResponse::builder().status(answer.status);
    for (name, value) in answer.headers {
        response = response.header(name, value);
    }
    response
        .body(Body::from(body))
        .unwrap_or_else(|_source| failure(StatusCode::INTERNAL_SERVER_ERROR))
}

fn failure(status: StatusCode) -> HttpResponse<Body> {
    HttpResponse::builder()
        .status(status)
        .body(Body::empty())
        .unwrap_or_else(|_source| HttpResponse::new(Body::empty()))
}

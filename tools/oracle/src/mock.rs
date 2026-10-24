use std::collections::{BTreeMap, VecDeque};
use std::io::Error;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::Response;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::oneshot;

use crate::case::UpstreamAnswer;
use crate::http::MAX_BODY;

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub(crate) struct ProviderRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
}

#[derive(Debug, Error)]
pub(crate) enum MockError {
    #[error("mock provider failed: {0}")]
    Io(#[from] Error),
    #[error("mock provider thread panicked")]
    Join,
}

struct Recording {
    answers: VecDeque<UpstreamAnswer>,
    requests: Vec<ProviderRequest>,
    failure: Option<Error>,
}

pub(crate) struct MockProvider {
    pub(crate) base_url: String,
    recording: Arc<Mutex<Recording>>,
    stop: Option<oneshot::Sender<()>>,
    pub(crate) thread: Option<JoinHandle<Result<(), Error>>>,
}

impl MockProvider {
    pub(crate) fn start(answers: &[UpstreamAnswer]) -> Result<Self, MockError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let recording = Arc::new(Mutex::new(Recording {
            answers: answers.iter().cloned().collect(),
            requests: Vec::new(),
            failure: None,
        }));
        let router = Router::new()
            .fallback(handle)
            .with_state(Arc::clone(&recording));
        let (stop, stopped) = oneshot::channel();
        let thread = thread::Builder::new().spawn(move || {
            runtime.block_on(async move {
                let socket = tokio::net::TcpListener::from_std(listener)?;
                axum::serve(socket, router)
                    .with_graceful_shutdown(async {
                        let _stopped = stopped.await;
                    })
                    .await
            })
        })?;
        Ok(Self {
            base_url: format!("http://{address}"),
            recording,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    pub(crate) fn finish(mut self) -> Result<Vec<ProviderRequest>, MockError> {
        self.shutdown()?;
        let mut recorded = self
            .recording
            .lock()
            .map_err(|error| Error::other(error.to_string()))?;
        if let Some(error) = recorded.failure.take() {
            return Err(MockError::Io(error));
        }
        Ok(std::mem::take(&mut recorded.requests))
    }

    fn shutdown(&mut self) -> Result<(), MockError> {
        if let Some(stop) = self.stop.take() {
            let _sent = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| MockError::Join)??;
        }
        Ok(())
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        let _shutdown = self.shutdown();
    }
}

async fn handle(
    State(state): State<Arc<Mutex<Recording>>>,
    request: Request,
) -> Result<Response<Body>, axum::http::StatusCode> {
    let (parts, body) = request.into_parts();
    if parts.method == "GET" && parts.uri.path() == "/v1/models" {
        return Ok(Response::new(Body::from(r#"{"object":"list","data":[]}"#)));
    }
    let bytes = tokio::time::timeout(Duration::from_secs(30), to_bytes(body, MAX_BODY))
        .await
        .map_err(|_| axum::http::StatusCode::REQUEST_TIMEOUT)?
        .map_err(|_| axum::http::StatusCode::PAYLOAD_TOO_LARGE)?;
    let mut recorded = state
        .lock()
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let result = reply(
        &mut recorded,
        ProviderRequest {
            method: parts.method.to_string(),
            path: parts.uri.to_string(),
            headers: parts
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.to_string(),
                        String::from_utf8_lossy(value.as_bytes()).into_owned(),
                    )
                })
                .collect(),
            body: serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        },
    );
    result.map_err(|error| {
        recorded.failure = Some(error);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })
}

fn reply(recorded: &mut Recording, request: ProviderRequest) -> Result<Response<Body>, Error> {
    recorded.requests.push(request);
    let answer = recorded
        .answers
        .pop_front()
        .ok_or_else(|| Error::other("provider answer sequence exhausted"))?;
    let body = if let Some(json) = answer.body {
        serde_json::to_vec(&json).map_err(Error::other)?
    } else {
        answer
            .events
            .unwrap_or_default()
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>()
            .into_bytes()
    };
    let mut response = Response::builder().status(answer.status);
    for (name, value) in answer.headers {
        response = response.header(name, value);
    }
    response.body(Body::from(body)).map_err(Error::other)
}

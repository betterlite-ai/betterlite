use axum::http::StatusCode;
use std::collections::BTreeMap;
use std::io::{Error, Write as _};
use std::net::TcpListener;
use std::str;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::{net, runtime, time};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::Response;
use futures_util::stream;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::OracleError;
use crate::case::UpstreamAnswer;
use crate::http::MAX_BODY;
use crate::routing_mock_scripts::{
    CallGuard, CallOutcome, MockScript, Outcome, ProviderCall, Recording, Ticket, redact,
};

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub(crate) struct ProviderRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
}

pub(crate) struct MockProvider {
    pub(crate) base_url: String,
    recording: Arc<Mutex<Recording>>,
    stop: Option<oneshot::Sender<()>>,
    pub(crate) thread: Option<JoinHandle<Result<(), Error>>>,
}

impl MockProvider {
    pub(crate) fn start(script: MockScript) -> Result<Self, OracleError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(OracleError::MockIo)?;
        listener
            .set_nonblocking(true)
            .map_err(OracleError::MockIo)?;
        let address = listener.local_addr().map_err(OracleError::MockIo)?;
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(OracleError::MockIo)?;
        let recording = Arc::new(Mutex::new(Recording {
            script,
            run: 0,
            barriers: BTreeMap::new(),
            trace: Vec::new(),
            failure: None,
        }));
        let router = Router::new()
            .fallback(handle)
            .with_state(Arc::clone(&recording));
        let (stop, stopped) = oneshot::channel();
        let shutdown = async {
            let _stopped = stopped.await;
        };
        let thread = thread::Builder::new()
            .spawn(move || {
                runtime.block_on(async move {
                    let socket = net::TcpListener::from_std(listener)?;
                    axum::serve(socket, router)
                        .with_graceful_shutdown(shutdown)
                        .await
                })
            })
            .map_err(OracleError::MockIo)?;
        Ok(Self {
            base_url: format!("http://{address}"),
            recording,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    pub(crate) fn finish_run(&self) -> Result<Vec<ProviderCall>, OracleError> {
        let mut recorded = self
            .recording
            .lock()
            .map_err(|error| OracleError::MockIo(Error::other(error.to_string())))?;
        recorded.finish_run().map_err(OracleError::MockIo)
    }

    pub(crate) fn finish(mut self) -> Result<Vec<ProviderCall>, OracleError> {
        self.shutdown()?;
        self.finish_run()
    }

    fn shutdown(&mut self) -> Result<(), OracleError> {
        if let Some(stop) = self.stop.take() {
            let _sent = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_source| OracleError::MockJoin)?
                .map_err(OracleError::MockIo)?;
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
) -> Result<Response<Body>, StatusCode> {
    let (parts, body) = request.into_parts();
    if parts.method == "GET" && parts.uri.path() == "/v1/models" {
        return Ok(Response::new(Body::from(r#"{"object":"list","data":[]}"#)));
    }
    let bytes = time::timeout(Duration::from_secs(30), to_bytes(body, MAX_BODY))
        .await
        .map_err(|_source| StatusCode::REQUEST_TIMEOUT)?
        .map_err(|_source| StatusCode::PAYLOAD_TOO_LARGE)?;
    let text = str::from_utf8(&bytes).map_err(|_source| StatusCode::BAD_REQUEST)?;
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
        .map_err(|_source| StatusCode::BAD_REQUEST)?;
    redact(&mut headers);
    let captured = ProviderRequest {
        method: parts.method.to_string(),
        path: parts.uri.to_string(),
        headers,
        body: serde_json::from_str(text).unwrap_or_else(|_source| Value::String(text.to_owned())),
    };
    let admitted = state
        .lock()
        .map_err(|_source| StatusCode::INTERNAL_SERVER_ERROR)?
        .admit(captured);
    let ticket = admitted.map_err(|error| fail(&state, error))?;
    play(state, ticket).await
}

async fn play(state: Arc<Mutex<Recording>>, ticket: Ticket) -> Result<Response<Body>, StatusCode> {
    let mut guard = CallGuard {
        state,
        run: ticket.run,
        index: ticket.index,
        outcome: CallOutcome::Cancelled,
    };
    let scripted = match (ticket.wait().await, ticket.then) {
        (false, _) => {
            guard.outcome = CallOutcome::DeadlineExceeded;
            None
        }
        (true, Outcome::Close) => {
            guard.outcome = CallOutcome::Closed;
            None
        }
        (true, Outcome::Answer(answer)) => {
            guard.outcome = CallOutcome::Answered {
                status: answer.status,
            };
            Some(answer)
        }
    };
    // a body that fails before its first frame makes hyper drop the connection without a status line
    let Some(answer) = scripted else {
        return Ok(Response::new(Body::from_stream(stream::iter([Err::<
            Vec<u8>,
            Error,
        >(
            Error::other("scripted connection close"),
        )]))));
    };
    reply(answer).map_err(|error| fail(&guard.state, error))
}

fn fail(state: &Mutex<Recording>, error: Error) -> StatusCode {
    if let Ok(mut recorded) = state.lock() {
        recorded.failure = Some(error);
    }
    StatusCode::INTERNAL_SERVER_ERROR
}

fn reply(answer: UpstreamAnswer) -> Result<Response<Body>, Error> {
    let body = if let Some(json) = answer.body {
        serde_json::to_vec(&json).map_err(Error::other)?
    } else {
        let mut bytes = Vec::new();
        for event in answer.events.unwrap_or_default() {
            writeln!(bytes, "data: {event}\n")?;
        }
        bytes
    };
    let mut response = Response::builder().status(answer.status);
    for (name, value) in answer.headers {
        response = response.header(name, value);
    }
    response.body(Body::from(body)).map_err(Error::other)
}

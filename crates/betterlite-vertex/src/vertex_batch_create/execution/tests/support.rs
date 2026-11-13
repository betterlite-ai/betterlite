use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::{HeaderMap, Method};
use secrecy::SecretString;
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use crate::cache_lookup::transport::CacheTransport;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

pub(super) const JOB: &str = r#"{"name":"projects/1/locations/us-central1/batchPredictionJobs/777","state":"JOB_STATE_PENDING","createTime":"2024-01-01T00:00:00.000000Z","inputConfig":{"gcsSource":{"uris":["gs://bucket/file.jsonl"]}}}"#;

#[derive(Clone)]
pub(super) enum Reply {
    Status { status: u16, body: String },
    Failure,
}

pub(super) fn reply(status: u16, body: &str) -> Reply {
    Reply::Status {
        status,
        body: body.to_owned(),
    }
}

#[derive(Clone)]
pub(super) struct Recorded {
    pub method: Method,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub deadline: Duration,
}

fn record(request: OutboundRequest) -> Recorded {
    let body = if let RequestBody::Json(bytes) = request.body {
        bytes
    } else {
        Vec::new()
    };
    Recorded {
        method: request.method,
        url: request.url.to_string(),
        headers: request.headers,
        body,
        deadline: request.deadline,
    }
}

pub(super) fn access(project: &str) -> Result<AccessToken, CredentialCacheError> {
    Ok(AccessToken {
        token: SecretString::from("tok-123"),
        project: project.to_owned(),
    })
}

pub(super) struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Recorded>>,
}

impl Scripted {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

impl CacheTransport for Scripted {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.requests.lock().unwrap().push(record(request));
        let reply = self.replies.lock().unwrap().pop_front().unwrap();
        ready(match reply {
            Reply::Status { status, body } => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: body.into_bytes(),
            }),
            Reply::Failure => Err(TransportError::Closed),
        })
    }
}

// answers only after the test opens the gate, so a call that blocked its thread would never see the gate open
pub(super) struct Gated {
    gate: Mutex<Option<oneshot::Receiver<()>>>,
    pub entered: Arc<AtomicBool>,
}

impl Gated {
    pub(super) fn new(gate: oneshot::Receiver<()>) -> Self {
        Self {
            gate: Mutex::new(Some(gate)),
            entered: Arc::default(),
        }
    }
}

impl CacheTransport for Gated {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.entered.store(true, Ordering::SeqCst);
        let gate = self.gate.lock().unwrap().take();
        async move {
            if let Some(gate) = gate {
                gate.await.ok();
            }
            Ok(UpstreamResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: JOB.as_bytes().to_vec(),
            })
        }
    }
}

pub(super) struct Stalling {
    pub base: String,
    _held: Arc<Mutex<Vec<tokio::net::TcpStream>>>,
    _tasks: JoinSet<()>,
}

// accepts connections and never answers, so a client with a read timeout fails first
pub(super) async fn stalling_server() -> Stalling {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let held: Arc<Mutex<Vec<tokio::net::TcpStream>>> = Arc::default();
    let keeper = Arc::clone(&held);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        while let Ok((stream, _peer)) = listener.accept().await {
            keeper.lock().unwrap().push(stream);
        }
    });
    Stalling {
        base,
        _held: held,
        _tasks: tasks,
    }
}

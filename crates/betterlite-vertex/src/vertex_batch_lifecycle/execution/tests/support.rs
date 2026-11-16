use std::collections::VecDeque;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use secrecy::SecretString;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::cache_lookup::transport::CacheTransport;
use crate::token_cache::probe::AccessToken;
use crate::vertex_batch_create::error::VertexBatchCreateError;
use crate::vertex_batch_create::fetch::ProtectedGet;

pub(super) const JOB: &str = r#"{"name":"projects/proj/locations/us-central1/batchPredictionJobs/777","state":"JOB_STATE_PENDING","createTime":"2024-01-01T00:00:00.000000Z","inputConfig":{"gcsSource":{"uris":["gs://bucket/file.jsonl"]}}}"#;
pub(super) const SECOND_JOB: &str = r#"{"name":"projects/proj/locations/us-central1/batchPredictionJobs/888","state":"JOB_STATE_RUNNING","createTime":"2024-01-02T00:00:00.000000Z"}"#;
pub(super) const CANCELLING: &str = r#"{"name":"projects/proj/locations/us-central1/batchPredictionJobs/777","state":"JOB_STATE_CANCELLING","createTime":"2024-01-01T00:00:00.000000Z","inputConfig":{"gcsSource":{"uris":["gs://bucket/file.jsonl"]}}}"#;

pub(super) fn page(jobs: &[&str], token: &str) -> String {
    let joined = jobs.join(",");
    if token.is_empty() {
        format!(r#"{{"batchPredictionJobs":[{joined}]}}"#)
    } else {
        format!(r#"{{"batchPredictionJobs":[{joined}],"nextPageToken":"{token}"}}"#)
    }
}

#[derive(Clone)]
pub(super) enum Reply {
    Status { status: u16, body: Vec<u8> },
    Failure,
}

pub(super) fn reply(status: u16, body: &str) -> Reply {
    reply_bytes(status, body.as_bytes())
}

pub(super) fn reply_bytes(status: u16, body: &[u8]) -> Reply {
    Reply::Status {
        status,
        body: body.to_vec(),
    }
}

#[derive(Clone)]
pub(super) struct Recorded {
    pub method: Method,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub deadline: Duration,
    // true when the call went through the SSRF-checked GET rather than the pooled client
    pub protected: bool,
}

pub(super) fn access(project: &str) -> AccessToken {
    AccessToken {
        token: SecretString::from("tok-123"),
        project: project.to_owned(),
    }
}

pub(super) struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Recorded>>,
}

impl Reply {
    fn answer(self) -> Result<UpstreamResponse, TransportError> {
        match self {
            Self::Status { status, body } => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            Self::Failure => Err(TransportError::Closed),
        }
    }
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

    fn record(&self, call: Recorded) -> Reply {
        self.requests.lock().unwrap().push(call);
        self.replies.lock().unwrap().pop_front().unwrap()
    }
}

impl CacheTransport for Scripted {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let body = if let RequestBody::Json(bytes) = request.body {
            bytes
        } else {
            Vec::new()
        };
        let reply = self.record(Recorded {
            method: request.method,
            url: request.url.to_string(),
            query: request.query,
            headers: request.headers,
            body,
            deadline: request.deadline,
            protected: false,
        });
        ready(reply.answer())
    }
}

impl ProtectedGet for Scripted {
    fn get(
        &self,
        url: &str,
        headers: &HeaderMap,
        deadline: Duration,
    ) -> impl Future<Output = Result<UpstreamResponse, VertexBatchCreateError>> + Send {
        let reply = self.record(Recorded {
            method: Method::GET,
            url: url.to_owned(),
            query: Vec::new(),
            headers: headers.clone(),
            body: Vec::new(),
            deadline,
            protected: true,
        });
        ready(
            reply
                .answer()
                .map_err(VertexBatchCreateError::from_transport),
        )
    }
}

// a transport whose answer never arrives, so the test can observe the caller's drop closing it
pub(super) struct Stalled {
    pub(super) dropped: Arc<AtomicBool>,
}

impl Stalled {
    pub(super) fn new() -> Self {
        Self {
            dropped: Arc::default(),
        }
    }
}

struct Silent {
    dropped: Arc<AtomicBool>,
}

impl Drop for Silent {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl Future for Silent {
    type Output = Result<UpstreamResponse, TransportError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl CacheTransport for Stalled {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        Silent {
            dropped: Arc::clone(&self.dropped),
        }
    }
}

// accepts connections and never answers, so only the client's own request deadline ends the call
pub(super) struct Hanging {
    pub(super) base: String,
    pub(super) _held: Arc<Mutex<Vec<TcpStream>>>,
    pub(super) _tasks: JoinSet<()>,
}

pub(super) async fn hanging_server() -> Hanging {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let held: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
    let keeper = Arc::clone(&held);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        while let Ok((stream, _peer)) = listener.accept().await {
            keeper.lock().unwrap().push(stream);
        }
    });
    Hanging {
        base,
        _held: held,
        _tasks: tasks,
    }
}

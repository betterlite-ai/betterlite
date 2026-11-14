use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use secrecy::SecretString;
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use crate::cache_lookup::transport::CacheTransport;
use crate::token_cache::probe::AccessToken;
use crate::vertex_batch_create::error::VertexBatchCreateError;
use crate::vertex_batch_create::fetch::ProtectedGet;

pub(super) const JOB: &str = r#"{"name":"projects/1/locations/us-central1/batchPredictionJobs/777","state":"JOB_STATE_PENDING","createTime":"2024-01-01T00:00:00.000000Z","inputConfig":{"gcsSource":{"uris":["gs://bucket/file.jsonl"]}}}"#;

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
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub deadline: Duration,
}

pub(super) fn access(project: &str) -> AccessToken {
    AccessToken {
        token: SecretString::from("tok-123"),
        project: project.to_owned(),
    }
}

pub(super) struct Scripted {
    pub(super) replies: Mutex<VecDeque<Reply>>,
    pub(super) requests: Mutex<Vec<Recorded>>,
    pub(super) gate: Mutex<Option<oneshot::Receiver<()>>>,
    pub(super) entered: Arc<AtomicBool>,
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
            gate: Mutex::new(None),
            entered: Arc::default(),
        }
    }

    // the create answers only after the test opens the gate, so a call that blocked its thread would never see it open
    pub(super) fn gated(self, gate: oneshot::Receiver<()>) -> Self {
        *self.gate.lock().unwrap() = Some(gate);
        self
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
        self.entered.store(true, Ordering::SeqCst);
        let release = self.gate.lock().unwrap().take();
        let body = if let RequestBody::Json(bytes) = request.body {
            bytes
        } else {
            Vec::new()
        };
        let reply = self.record(Recorded {
            method: request.method,
            url: request.url.to_string(),
            headers: request.headers,
            body,
            deadline: request.deadline,
        });
        async move {
            if let Some(open_gate) = release {
                assert!(open_gate.await.is_ok(), "the test opened the gate");
            }
            reply.answer()
        }
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
            headers: headers.clone(),
            body: Vec::new(),
            deadline,
        });
        ready(
            reply
                .answer()
                .map_err(VertexBatchCreateError::from_transport),
        )
    }
}

pub(super) struct Stalling {
    pub(super) base: String,
    pub(super) _held: Arc<Mutex<Vec<TcpStream>>>,
    pub(super) _tasks: JoinSet<()>,
}

pub(super) const STALLED_BODY_HEAD: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 64\r\n\r\n";

// accepts connections and never answers, or answers this head and stalls the body, so a client's own read timeout fails first
pub(super) async fn stalled_server(head: Option<&'static [u8]>) -> Stalling {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let accepted: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
    let keeper = Arc::clone(&accepted);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        while let Ok((mut stream, _peer)) = listener.accept().await {
            if let Some(response_head) = head {
                let _written = stream.write_all(response_head).await;
            }
            keeper.lock().unwrap().push(stream);
        }
    });
    Stalling {
        base,
        _held: accepted,
        _tasks: tasks,
    }
}

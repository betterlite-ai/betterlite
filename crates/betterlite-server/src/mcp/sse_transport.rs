use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode, header};
use chrono::Utc;
use futures_channel::mpsc;
use futures_util::SinkExt as _;
use futures_util::stream::Stream;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rmcp::model::ClientJsonRpcMessage;
use rmcp::service::TxJsonRpcMessage;
use rmcp::{RoleServer, ServerHandler, serve_server};
use tokio::task::JoinSet;
use tokio::time::{Interval, interval};
use uuid::Uuid;

use super::stream_transport::wire;

mod tests;

// MCP's DEFAULT_MAX_REQUEST_BODY_SIZE, the limit the SDK's message endpoint enforces
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
const SERVICE_CHANNEL_CAPACITY: usize = 32;
// sse-starlette's DEFAULT_PING_INTERVAL
const PING_INTERVAL: Duration = Duration::from_secs(15);
// Python's quote leaves the unreserved characters and "/" alone
const QUOTE_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b'/');

const SSE_HEADERS: [(&str, &str); 4] = [
    ("content-type", "text/event-stream; charset=utf-8"),
    ("cache-control", "no-store"),
    ("connection", "keep-alive"),
    ("x-accel-buffering", "no"),
];

pub struct SseTransport {
    endpoint: String,
    sessions: Arc<Mutex<HashMap<Uuid, Session>>>,
}

struct Session {
    owner: String,
    service: mpsc::Sender<ClientJsonRpcMessage>,
    // holding the tasks here stops them with the session, so only the drop matters
    _tasks: JoinSet<()>,
}

impl SseTransport {
    #[must_use]
    pub fn new(endpoint: &str) -> Self {
        let path = if endpoint.starts_with('/') {
            endpoint.to_owned()
        } else {
            format!("/{endpoint}")
        };
        Self {
            endpoint: path,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn connect<H>(&self, root_path: &str, owner: &str, handler: H) -> Response<Body>
    where
        H: ServerHandler,
    {
        let session_id = Uuid::new_v4();
        let (to_service, from_client) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
        let (to_client, client_events) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            match serve_server(handler, (to_client, from_client)).await {
                Ok(running) => drop(running.waiting().await),
                Err(error) => tracing::warn!(%error, "an MCP legacy SSE session failed"),
            }
        });
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                session_id,
                Session {
                    owner: owner.to_owned(),
                    service: to_service,
                    _tasks: tasks,
                },
            );
        let full_path = format!("{}{}", root_path.trim_end_matches('/'), self.endpoint);
        let client_uri = format!(
            "{}?session_id={}",
            utf8_percent_encode(&full_path, QUOTE_SAFE),
            session_id.simple()
        );
        let endpoint_event = Bytes::from(format!("event: endpoint\r\ndata: {client_uri}\r\n\r\n"));
        sse_response(Body::from_stream(SseStream {
            first: Some(endpoint_event),
            client_events,
            ping: ping_interval(),
            _release: SessionRelease {
                sessions: Arc::clone(&self.sessions),
                id: session_id,
            },
        }))
    }

    pub async fn post(
        &self,
        query: Option<&str>,
        headers: &HeaderMap,
        owner: &str,
        body: Body,
    ) -> Response<Body> {
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_type
            .to_ascii_lowercase()
            .starts_with("application/json")
        {
            return plain_response(StatusCode::BAD_REQUEST, "Invalid Content-Type header");
        }
        let Some(raw_session_id) = session_id_param(query) else {
            return plain_response(StatusCode::BAD_REQUEST, "session_id is required");
        };
        let Some(session_id) = parse_session_id(&raw_session_id) else {
            return plain_response(StatusCode::BAD_REQUEST, "Invalid session ID");
        };
        let Some(mut service) = self.session_service(&session_id, owner) else {
            return plain_response(StatusCode::NOT_FOUND, "Could not find session");
        };
        let Ok(bytes) = to_bytes(body, MAX_REQUEST_BYTES).await else {
            return plain_response(StatusCode::PAYLOAD_TOO_LARGE, "Request body too large");
        };
        let Ok(message) = wire::decode_message(&bytes) else {
            return plain_response(StatusCode::BAD_REQUEST, "Could not parse message");
        };
        if service.send(message).await.is_err() {
            // the service has ended, so the session can no longer receive a message
            self.sessions
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&session_id);
            return plain_response(StatusCode::NOT_FOUND, "Could not find session");
        }
        plain_response(StatusCode::ACCEPTED, "Accepted")
    }

    fn session_service(
        &self,
        id: &Uuid,
        owner: &str,
    ) -> Option<mpsc::Sender<ClientJsonRpcMessage>> {
        let sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let session = sessions.get(id)?;
        let owner_matches = session.owner == owner;
        let service = session.service.clone();
        drop(sessions);
        owner_matches.then_some(service)
    }
}

fn session_id_param(query: Option<&str>) -> Option<String> {
    let raw_query = query?;
    let mut session_id = None;
    for (name, value) in form_urlencoded::parse(raw_query.as_bytes()) {
        if name.as_ref() == "session_id" {
            session_id = Some(value.into_owned());
        }
    }
    session_id
}

fn parse_session_id(raw: &str) -> Option<Uuid> {
    // Python's UUID(hex=...) drops a urn:uuid: prefix, braces and every hyphen before parsing
    let without_prefix = raw.replace("urn:", "").replace("uuid:", "");
    let trimmed = without_prefix.trim_matches(|character| character == '{' || character == '}');
    let hex = trimmed.replace('-', "");
    if hex.len() != 32 {
        return None;
    }
    Uuid::parse_str(&hex).ok()
}

fn ping_interval() -> Interval {
    let mut ping = interval(PING_INTERVAL);
    ping.reset();
    ping
}

fn plain_response(status: StatusCode, text: &'static str) -> Response<Body> {
    let mut response = Response::new(Body::from(text));
    *response.status_mut() = status;
    response
}

fn sse_response(body: Body) -> Response<Body> {
    let mut response = Response::new(body);
    for (name, value) in SSE_HEADERS {
        response.headers_mut().insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    response
}

struct SseStream {
    first: Option<Bytes>,
    client_events: mpsc::Receiver<TxJsonRpcMessage<RoleServer>>,
    ping: Interval,
    // dropping the stream releases the session, so only the drop matters
    _release: SessionRelease,
}

struct SessionRelease {
    sessions: Arc<Mutex<HashMap<Uuid, Session>>>,
    id: Uuid,
}

impl Stream for SseStream {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Some(event) = this.first.take() {
            return Poll::Ready(Some(Ok(event)));
        }
        match Pin::new(&mut this.client_events).poll_next(cx) {
            Poll::Ready(Some(message)) => return Poll::Ready(Some(Ok(wire::sse_event(&message)))),
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
        }
        match this.ping.poll_tick(cx) {
            Poll::Ready(_) => Poll::Ready(Some(Ok(Bytes::from(format!(
                ": ping - {}\r\n\r\n",
                Utc::now().format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            ))))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for SessionRelease {
    fn drop(&mut self) {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

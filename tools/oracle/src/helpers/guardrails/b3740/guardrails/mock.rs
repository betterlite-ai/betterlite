use std::collections::VecDeque;
use std::net::TcpListener;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{Builder, JoinHandle};

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::header::{ETAG, HeaderValue};
use axum::response::{IntoResponse as _, Response};
use betterlite_oracle_helper_api::error::HelperError;
use serde::Deserialize;
use tokio::net::TcpListener as AsyncListener;
use tokio::runtime::Builder as RuntimeBuilder;

use super::super::wire::Reply;

const MAX_BODY_BYTES: usize = 1_048_576;
const EXHAUSTED_BODY: &str = "{}";
const TOKEN_BODY: &str = r#"{"access_token":"tok","expires_in":3600}"#;
const SCOPES_BODY: &str = r#"{"value":[]}"#;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct Scan {
    pub(super) activity: String,
    pub(super) data: String,
}

struct QueuedReply {
    status: StatusCode,
    body: String,
}

struct MockState {
    replies: Mutex<VecDeque<QueuedReply>>,
    scans: Arc<Mutex<Vec<Scan>>>,
}

pub(super) struct GraphMock {
    pub(super) base: String,
    scans: Arc<Mutex<Vec<Scan>>>,
    _server: JoinHandle<()>,
}

impl GraphMock {
    pub(super) fn scans(&self) -> Vec<Scan> {
        self.scans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Deserialize)]
struct ProcessBody {
    #[serde(rename = "contentToProcess")]
    content_to_process: ContentToProcess,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContentToProcess {
    content_entries: Vec<ContentEntry>,
    activity_metadata: ActivityMetadata,
}

#[derive(Deserialize)]
struct ContentEntry {
    content: TextContent,
}

#[derive(Deserialize)]
struct TextContent {
    data: String,
}

#[derive(Deserialize)]
struct ActivityMetadata {
    activity: String,
}

fn respond(reply: &QueuedReply) -> Response {
    let mut response = (reply.status, reply.body.clone()).into_response();
    response
        .headers_mut()
        .insert(ETAG, HeaderValue::from_static("\"etag-1\""));
    response
}

fn record_scan(state: &MockState, bytes: &[u8]) {
    let Ok(process) = serde_json::from_slice::<ProcessBody>(bytes) else {
        return;
    };
    let Some(entry) = process.content_to_process.content_entries.first() else {
        return;
    };
    state
        .scans
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Scan {
            activity: process
                .content_to_process
                .activity_metadata
                .activity
                .clone(),
            data: entry.content.data.clone(),
        });
}

async fn serve(State(state): State<Arc<MockState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_BODY_BYTES).await.unwrap_or_default();
    let path = parts.uri.path().to_owned();
    if path.ends_with("/processContent") {
        record_scan(&state, &bytes);
        let reply = state
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| QueuedReply {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                body: EXHAUSTED_BODY.to_owned(),
            });
        return respond(&reply);
    }
    if path.ends_with("/protectionScopes/compute") {
        return respond(&QueuedReply {
            status: StatusCode::OK,
            body: SCOPES_BODY.to_owned(),
        });
    }
    respond(&QueuedReply {
        status: StatusCode::OK,
        body: TOKEN_BODY.to_owned(),
    })
}

fn queue(replies: &[Reply]) -> Result<VecDeque<QueuedReply>, Box<HelperError>> {
    let mut queued = VecDeque::new();
    for reply in replies {
        let status =
            StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_string(&reply.body)?;
        queued.push_back(QueuedReply { status, body });
    }
    Ok(queued)
}

fn serve_in_thread(
    listener: TcpListener,
    router: Router,
) -> Result<JoinHandle<()>, Box<HelperError>> {
    let server = Builder::new()
        .name("purview-oracle-mock".to_owned())
        .spawn(move || {
            let runtime = match RuntimeBuilder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::error!("purview mock runtime: {error}");
                    return;
                }
            };
            runtime.block_on(async move {
                let listener = match AsyncListener::from_std(listener) {
                    Ok(listener) => listener,
                    Err(error) => {
                        tracing::error!("purview mock listener: {error}");
                        return;
                    }
                };
                if let Err(error) = axum::serve(listener, router).await {
                    tracing::error!("purview mock server: {error}");
                }
            });
        })?;
    Ok(server)
}

pub(super) fn spawn_mock(replies: &[Reply]) -> Result<GraphMock, Box<HelperError>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    let scans = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::new(MockState {
        replies: Mutex::new(queue(replies)?),
        scans: Arc::clone(&scans),
    });
    let router = Router::new().fallback(serve).with_state(state);
    let server = serve_in_thread(listener, router)?;
    Ok(GraphMock {
        base: format!("http://{address}"),
        scans,
        _server: server,
    })
}

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
use serde_json::Value;
use tokio::net::TcpListener as AsyncListener;
use tokio::runtime::Builder as RuntimeBuilder;

use super::super::wire::Reply;

const MAX_BODY_BYTES: usize = 0x0010_0000;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct Scan {
    pub(super) activity: String,
    pub(super) data: String,
}

struct MockState {
    replies: Mutex<VecDeque<(StatusCode, String)>>,
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

// The oracle compares the activity and the scanned text, which are the only fields the process body carries here.
fn scan_of(bytes: &[u8]) -> Option<Scan> {
    let process: Value = serde_json::from_slice(bytes).ok()?;
    let text = process.pointer("/contentToProcess/contentEntries/0/content/data")?;
    let activity = process.pointer("/contentToProcess/activityMetadata/activity")?;
    Some(Scan {
        activity: activity.as_str()?.to_owned(),
        data: text.as_str()?.to_owned(),
    })
}

fn respond(status: StatusCode, body: String) -> Response {
    let mut response = (status, body).into_response();
    let _previous = response
        .headers_mut()
        .insert(ETAG, HeaderValue::from_static("\"etag-1\""));
    response
}

async fn serve(State(state): State<Arc<MockState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_BODY_BYTES).await.unwrap_or_default();
    let path = parts.uri.path().to_owned();
    if path.ends_with("/processContent") {
        if let Some(scan) = scan_of(&bytes) {
            state
                .scans
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(scan);
        }
        let reply = state
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, "{}".to_owned()));
        return respond(reply.0, reply.1);
    }
    if path.ends_with("/protectionScopes/compute") {
        return respond(StatusCode::OK, r#"{"value":[]}"#.to_owned());
    }
    respond(
        StatusCode::OK,
        r#"{"access_token":"tok","expires_in":3600}"#.to_owned(),
    )
}

fn queue(replies: &[Reply]) -> Result<VecDeque<(StatusCode, String)>, Box<HelperError>> {
    let mut queued = VecDeque::new();
    for reply in replies {
        let status =
            StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        queued.push_back((status, serde_json::to_string(&reply.body)?));
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
            let Ok(runtime) = RuntimeBuilder::new_current_thread().enable_all().build() else {
                return;
            };
            runtime.block_on(async move {
                let Ok(async_listener) = AsyncListener::from_std(listener) else {
                    return;
                };
                if let Err(error) = axum::serve(async_listener, router).await {
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

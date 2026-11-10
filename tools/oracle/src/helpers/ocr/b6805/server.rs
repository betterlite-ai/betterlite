use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::failure;
use crate::helpers::HelperError;

const BODY_LIMIT: usize = 1024 * 1024;

pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct Captured {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
}

#[derive(Clone)]
struct Mock {
    base: String,
    replies: Arc<[Reply]>,
    captured: Arc<Mutex<Vec<Captured>>>,
    index: Arc<Mutex<usize>>,
}

pub(super) struct Server {
    pub(super) base: String,
    captured: Arc<Mutex<Vec<Captured>>>,
    tasks: JoinSet<()>,
}

impl Server {
    pub(super) async fn start(replies: Vec<Reply>) -> Result<Self, Box<HelperError>> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| failure(&error))?;
        let address = listener.local_addr().map_err(|error| failure(&error))?;
        let base = format!("http://{address}");
        let captured = Arc::new(Mutex::new(Vec::new()));
        let mock = Mock {
            base: base.clone(),
            replies: Arc::from(replies),
            captured: Arc::clone(&captured),
            index: Arc::new(Mutex::new(0)),
        };
        let application = Router::new().fallback(answer).with_state(mock);
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            // The server only stops when the helper drops it and aborts the task.
            let _served = axum::serve(listener, application).await;
        });
        Ok(Self {
            base,
            captured,
            tasks,
        })
    }

    pub(super) fn finish(mut self) -> Vec<Captured> {
        self.tasks.abort_all();
        self.captured
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }
}

async fn answer(State(mock): State<Mock>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(bytes) = to_bytes(body, BODY_LIMIT).await else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let path = parts.uri.query().map_or_else(
        || parts.uri.path().to_owned(),
        |query| format!("{}?{query}", parts.uri.path()),
    );
    let headers = pair_headers(&parts.headers);
    mock.captured
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Captured {
            method: parts.method.as_str().to_owned(),
            path,
            headers,
            body: bytes.to_vec(),
        });
    match next_reply(&mock) {
        Some(reply) => render(&mock.base, reply).into_response(),
        None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn pair_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut pairs = BTreeMap::new();
    for (name, value) in headers {
        if let Ok(text) = value.to_str() {
            let _previous = pairs.insert(name.as_str().to_ascii_lowercase(), text.to_owned());
        }
    }
    pairs
}

fn next_reply(mock: &Mock) -> Option<&Reply> {
    let mut index = mock.index.lock().unwrap_or_else(PoisonError::into_inner);
    let chosen = mock.replies.get(*index).or_else(|| mock.replies.last());
    *index = index.saturating_add(1);
    chosen
}

fn render(base: &str, reply: &Reply) -> Response {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = Response::builder().status(status);
    for (name, value) in &reply.headers {
        let resolved = if value.starts_with('/') {
            format!("{base}{value}")
        } else {
            value.clone()
        };
        response = response.header(name, resolved);
    }
    response
        .body(axum::body::Body::from(reply.body.clone()))
        .unwrap_or_else(|_error| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

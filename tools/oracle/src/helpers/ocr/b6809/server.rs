use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use bytes::Bytes;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::failure;
use crate::helpers::HelperError;

const BODY_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct Captured {
    pub(super) path: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
}

#[derive(Clone)]
struct Mock {
    answer: Bytes,
    seen: Arc<Mutex<Option<Captured>>>,
}

pub(super) struct Server {
    pub(super) base: String,
    seen: Arc<Mutex<Option<Captured>>>,
    tasks: JoinSet<()>,
}

impl Server {
    pub(super) async fn start(answer: Vec<u8>) -> Result<Self, Box<HelperError>> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| failure(&error))?;
        let address = listener.local_addr().map_err(|error| failure(&error))?;
        let seen = Arc::new(Mutex::new(None));
        let mock = Mock {
            answer: Bytes::from(answer),
            seen: Arc::clone(&seen),
        };
        let application = Router::new().fallback(answer_request).with_state(mock);
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            // The helper aborts the task once the reply was served, so the serve result is not read.
            let _served = axum::serve(listener, application).await;
        });
        Ok(Self {
            base: format!("http://{address}"),
            seen,
            tasks,
        })
    }

    pub(super) fn finish(mut self) -> Result<Captured, Box<HelperError>> {
        self.tasks.abort_all();
        let captured = {
            let seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
            seen.clone()
        };
        captured.ok_or_else(|| failure("the mock provider received no request"))
    }
}

async fn answer_request(State(mock): State<Mock>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(bytes) = to_bytes(body, BODY_LIMIT).await else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let path = parts.uri.query().map_or_else(
        || parts.uri.path().to_owned(),
        |query| format!("{}?{query}", parts.uri.path()),
    );
    *mock.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(Captured {
        path,
        headers: pair_headers(&parts.headers),
        body: bytes.to_vec(),
    });
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(mock.answer.clone()))
        .unwrap_or_else(|_error| StatusCode::INTERNAL_SERVER_ERROR.into_response())
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

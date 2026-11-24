use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Request as IncomingRequest;
use axum::extract::State;
use axum::response::Response;
use http::header::{CONTENT_TYPE, HeaderMap, LOCATION};
use http::{Method, StatusCode};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

const BODY_LIMIT: usize = 0x0010_0000;

#[derive(Clone, Debug)]
pub(super) struct Recorded {
    pub(super) method: Method,
    pub(super) uri: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

#[derive(Clone)]
struct ServerState {
    requests: Arc<Mutex<Vec<Recorded>>>,
    reply: Arc<dyn Fn(&Recorded) -> Response + Send + Sync>,
}

pub(super) struct TestServer {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<Recorded>>>,
    _tasks: JoinSet<()>,
}

impl TestServer {
    pub(super) async fn start<Reply>(reply: Reply) -> Self
    where
        Reply: Fn(&Recorded) -> Response + Send + Sync + 'static,
    {
        let requests: Arc<Mutex<Vec<Recorded>>> = Arc::default();
        let state = ServerState {
            requests: Arc::clone(&requests),
            reply: Arc::new(reply),
        };
        let app = Router::new().fallback(record).with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            address,
            requests,
            _tasks: tasks,
        }
    }

    pub(super) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    pub(super) fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

async fn record(State(state): State<ServerState>, request: IncomingRequest) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, BODY_LIMIT).await.unwrap();
    let recorded = Recorded {
        method: parts.method,
        uri: parts.uri.to_string(),
        headers: parts.headers,
        body: bytes.to_vec(),
    };
    state
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(recorded.clone());
    (state.reply)(&recorded)
}

pub(super) fn ok_json(body: &str) -> Response {
    status_with_body(200, body)
}

pub(super) fn status_with_body(status: u16, body: &str) -> Response {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap())
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

pub(super) fn status(status: u16) -> Response {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap())
        .body(Body::empty())
        .unwrap()
}

pub(super) fn redirect(status: u16, location: &str) -> Response {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap())
        .header(LOCATION, location)
        .body(Body::empty())
        .unwrap()
}

pub(super) fn header_value(recorded: &Recorded, name: &str) -> Option<String> {
    recorded
        .headers
        .get(name)
        .map(|value| value.to_str().unwrap().to_owned())
}

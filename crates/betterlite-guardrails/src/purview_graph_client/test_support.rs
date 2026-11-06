#![cfg(test)]

use std::cell::Cell;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse as _, Response};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use uuid::Uuid;

use super::{CacheState, PurviewGuardrailBase};

pub(super) const START_SECONDS: u64 = 1_000_000;

thread_local! {
    pub(super) static NOW_SECONDS: Cell<u64> = const { Cell::new(START_SECONDS) };
}

pub(super) fn clock() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS.get()))
        .unwrap()
}

pub(super) fn clock_at(seconds: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .unwrap()
}

#[derive(Clone)]
pub(super) struct RecordedRequest {
    pub(super) path: String,
    pub(super) headers: HeaderMap,
    pub(super) body: String,
}

pub(super) struct MockReply {
    pub(super) status: StatusCode,
    pub(super) headers: Vec<(&'static str, &'static str)>,
    pub(super) body: String,
}

pub(super) type Responder = fn(&RecordedRequest) -> MockReply;

struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    responder: Responder,
}

pub(super) struct MockGraph {
    pub(super) base: String,
    pub(super) requests: Arc<Mutex<Vec<RecordedRequest>>>,
    _server: JoinSet<io::Result<()>>,
}

impl MockGraph {
    pub(super) fn count(&self, path_suffix: &str) -> usize {
        let requests = self.requests.lock().unwrap();
        requests
            .iter()
            .filter(|request| request.path.ends_with(path_suffix))
            .count()
    }
}

pub(super) fn json_reply(body: &str) -> MockReply {
    MockReply {
        status: StatusCode::OK,
        headers: Vec::new(),
        body: body.to_owned(),
    }
}

pub(super) fn standard_reply(request: &RecordedRequest) -> MockReply {
    if request.path.ends_with("/token") {
        return json_reply(r#"{"access_token":"tok","expires_in":3600}"#);
    }
    if request.path.ends_with("/compute") {
        return MockReply {
            status: StatusCode::OK,
            headers: vec![("etag", "\"etag-1\"")],
            body: r#"{"value":[{"id":"scope-1"}]}"#.to_owned(),
        };
    }
    json_reply(r#"{"id":"processed"}"#)
}

async fn record(State(state): State<Arc<MockState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 1024 * 1024).await.unwrap();
    let recorded = RecordedRequest {
        path: parts.uri.path().to_owned(),
        headers: parts.headers,
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    };
    let reply = (state.responder)(&recorded);
    state.requests.lock().unwrap().push(recorded);
    let mut headers = HeaderMap::new();
    for (name, value) in reply.headers {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_static(value),
        );
    }
    (reply.status, headers, reply.body).into_response()
}

pub(super) async fn start(responder: Responder) -> MockGraph {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::new(MockState {
        requests: Arc::clone(&requests),
        responder,
    });
    let router = Router::new().fallback(record).with_state(state);
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    MockGraph {
        base,
        requests,
        _server: server,
    }
}

pub(super) fn guardrail(base: &str) -> PurviewGuardrailBase {
    PurviewGuardrailBase {
        tenant_id: "tenant-1".to_owned(),
        client_id: "client-1".to_owned(),
        client_secret: SecretString::from("secret-1"),
        purview_app_name: "LiteLLM".to_owned(),
        user_id_field: "user_id".to_owned(),
        timeout: Duration::from_secs(5),
        client: reqwest::Client::default(),
        graph_api_base: base.to_owned(),
        login_authority: base.to_owned(),
        clock,
        new_id: Uuid::nil,
        scope_and_token_cache_lock: Mutex::new(CacheState::default()),
    }
}

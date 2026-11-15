#![cfg(test)]

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

use crate::purview_dlp_evaluation::{MicrosoftPurviewDLPGuardrail, PurviewDlpGuardrailParams};

const NOW_SECONDS: u64 = 1_000_000;
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) headers: Vec<(&'static str, &'static str)>,
    pub(super) body: String,
}

#[derive(Clone)]
pub(super) struct SeenRequest {
    pub(super) path: String,
    pub(super) headers: HeaderMap,
    pub(super) body: String,
}

#[derive(Clone)]
pub(super) struct Captured(pub(super) Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct MockState {
    replies: Vec<(&'static str, Reply)>,
    seen: Arc<Mutex<Vec<SeenRequest>>>,
}

pub(super) struct GraphMock {
    pub(super) base: String,
    pub(super) seen: Arc<Mutex<Vec<SeenRequest>>>,
    _server: JoinSet<io::Result<()>>,
}

impl GraphMock {
    pub(super) fn count(&self, suffix: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.path.ends_with(suffix))
            .count()
    }
}

impl Reply {
    pub(super) fn ok(body: &str) -> Self {
        Self {
            status: StatusCode::OK,
            headers: Vec::new(),
            body: body.to_owned(),
        }
    }
}

fn fixed_clock() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .unwrap()
}

async fn serve(State(state): State<Arc<MockState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_BODY_BYTES).await.unwrap();
    let path = parts.uri.path().to_owned();
    let recorded = SeenRequest {
        path: path.clone(),
        headers: parts.headers.clone(),
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    };
    state.seen.lock().unwrap().push(recorded);
    let Some((_suffix, reply)) = state
        .replies
        .iter()
        .find(|(suffix, _reply)| path.ends_with(*suffix))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut headers = HeaderMap::new();
    for (name, value) in &reply.headers {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_static(value),
        );
    }
    (reply.status, headers, reply.body.clone()).into_response()
}

pub(super) async fn start(replies: Vec<(&'static str, Reply)>) -> GraphMock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::new(MockState {
        replies,
        seen: Arc::clone(&seen),
    });
    let router = Router::new().fallback(serve).with_state(state);
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    GraphMock {
        base,
        seen,
        _server: server,
    }
}

pub(super) fn standard_replies() -> Vec<(&'static str, Reply)> {
    vec![
        (
            "/token",
            Reply::ok(r#"{"access_token":"tok","expires_in":3600}"#),
        ),
        (
            "/protectionScopes/compute",
            Reply {
                status: StatusCode::OK,
                headers: vec![("etag", "\"etag-1\"")],
                body: r#"{"value":[{"id":"scope-1"}]}"#.to_owned(),
            },
        ),
        ("/processContent", Reply::ok(r#"{"policyActions":[]}"#)),
    ]
}

pub(super) fn guardrail_at(base: &str) -> MicrosoftPurviewDLPGuardrail {
    let params = PurviewDlpGuardrailParams {
        guardrail_name: "purview-test".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        client_id: "client-1".to_owned(),
        client_secret: SecretString::from("secret-1"),
        purview_app_name: None,
        user_id_field: None,
        timeout: Some(Duration::from_secs(5)),
    };
    let mut guardrail = MicrosoftPurviewDLPGuardrail::new(params, reqwest::Client::default());
    guardrail.base.graph_api_base = base.to_owned();
    guardrail.base.login_authority = base.to_owned();
    guardrail.base.clock = fixed_clock;
    guardrail.base.new_id = Uuid::nil;
    guardrail
}

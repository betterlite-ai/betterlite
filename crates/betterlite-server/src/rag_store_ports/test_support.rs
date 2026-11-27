#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{HeaderName, HeaderValue};
use axum::http::{HeaderMap as HttpHeaders, Method, StatusCode, Uri};
use axum::response::{IntoResponse as _, Response};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[derive(Clone)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
}

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
}

impl Reply {
    pub(crate) fn json(body: &str) -> Self {
        Self {
            status: StatusCode::OK,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: body.to_owned(),
        }
    }

    #[must_use]
    pub(crate) fn with_status(mut self, status: u16) -> Self {
        self.status = StatusCode::from_u16(status).expect("the fixture status is valid");
        self
    }
}

#[derive(Clone)]
struct Exchange {
    reply: Reply,
    seen: Arc<Mutex<Vec<Seen>>>,
}

pub(crate) fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the proxy policy resolves");
    ProxiedClient::outbound(&Arc::new(policy)).expect("the client builds")
}

// Runs the scenario against a live server that records each request and answers with one reply
pub(crate) async fn with_server<T, F, Fut>(reply: Reply, scenario: F) -> (T, Vec<Seen>)
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = T>,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the test listener binds");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("the listener has an address")
    );
    let state = Exchange {
        reply,
        seen: Arc::default(),
    };
    let seen = Arc::clone(&state.seen);
    let app = Router::new().fallback(answer).with_state(state);
    let (stop, stopped) = oneshot::channel::<()>();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        stopped.await.unwrap_or_default();
    });
    let run = async move {
        let outcome = scenario(base).await;
        stop.send(()).unwrap_or_default();
        outcome
    };
    let (outcome, joined) = tokio::join!(run, server);
    joined.expect("the test server serves");
    let recorded = seen
        .lock()
        .expect("the fixture lock is not poisoned")
        .clone();
    (outcome, recorded)
}

async fn answer(
    State(state): State<Exchange>,
    method: Method,
    headers: HttpHeaders,
    uri: Uri,
    body: Bytes,
) -> Response {
    record(&state, &method, &headers, &uri, &body);
    let mut response_headers = HttpHeaders::new();
    for (name, value) in &state.reply.headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).expect("the fixture name is valid");
        let header_value = HeaderValue::from_str(value).expect("the fixture value is valid");
        response_headers.insert(header_name, header_value);
    }
    (
        state.reply.status,
        response_headers,
        state.reply.body.clone(),
    )
        .into_response()
}

fn record(state: &Exchange, method: &Method, headers: &HttpHeaders, uri: &Uri, body: &Bytes) {
    let mut seen_headers = BTreeMap::new();
    for (name, value) in headers {
        if let Ok(text) = value.to_str() {
            seen_headers.insert(name.as_str().to_owned(), text.to_owned());
        }
    }
    state
        .seen
        .lock()
        .expect("the fixture lock is not poisoned")
        .push(Seen {
            method: method.as_str().to_owned(),
            path: uri.path().to_owned(),
            headers: seen_headers,
            body: String::from_utf8(body.to_vec()).unwrap_or_default(),
        });
}

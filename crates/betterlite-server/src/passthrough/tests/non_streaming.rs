use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::routing::any;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use http::Method;
use http::header::{HeaderMap, HeaderName};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use url::Url;

mod passthrough_non_streaming_http_handler;

struct Captured {
    method: Method,
    uri: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

#[derive(Clone)]
struct Seen(Arc<Mutex<Vec<Captured>>>);

async fn capture(State(seen): State<Seen>, request: Request) -> StatusCode {
    let method = request.method().clone();
    let uri = request.uri().to_string();
    let headers = request.headers().clone();
    let body = to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    seen.0.lock().unwrap().push(Captured {
        method,
        uri,
        headers,
        body,
    });
    StatusCode::OK
}

async fn upstream() -> (Url, Seen, JoinSet<()>) {
    let seen = Seen(Arc::new(Mutex::new(Vec::new())));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let application = Router::new()
        .route("/capture", any(capture))
        .with_state(seen.clone());
    let mut server = JoinSet::new();
    server.spawn(async move { axum::serve(listener, application).await.unwrap() });
    (
        Url::parse(&format!("http://{address}/capture")).unwrap(),
        seen,
        server,
    )
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn header_text(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(HeaderName::from_static(name))
        .map(|value| value.to_str().unwrap().to_owned())
}

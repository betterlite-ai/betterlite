use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::onyx::OnyxGuardrail;
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;
use uuid::Uuid;

pub(crate) const API_KEY: &str = "oracle-key";

#[derive(Debug)]
pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct MockOnyx {
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<io::Result<()>>,
}

pub(crate) async fn mock_onyx(status: StatusCode, reply: String) -> MockOnyx {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_by_handler = Arc::clone(&seen);
    let answer = move |uri: Uri, bytes: Bytes| {
        let recorder = Arc::clone(&seen_by_handler);
        let reply_for_response = reply.clone();
        async move {
            recorder.lock().unwrap().push(Seen {
                path: uri.path().to_owned(),
                body: bytes.to_vec(),
            });
            (
                status,
                [("content-type", "application/json")],
                reply_for_response,
            )
        }
    };
    let (base, server) = serve(Router::new().fallback(answer)).await;
    MockOnyx {
        base,
        seen,
        _server: server,
    }
}

pub(crate) async fn silent_onyx() -> MockOnyx {
    let answer = |_uri: Uri, _bytes: Bytes| async {
        sleep(Duration::from_secs(30)).await;
        (StatusCode::OK, String::from("{}"))
    };
    let (base, server) = serve(Router::new().fallback(answer)).await;
    MockOnyx {
        base,
        seen: Arc::new(Mutex::new(Vec::new())),
        _server: server,
    }
}

async fn serve(router: Router) -> (String, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

pub(crate) fn fixed_id() -> Uuid {
    Uuid::from_u128(1)
}

pub(crate) fn guardrail_with_timeout(base: &str, timeout: Duration) -> OnyxGuardrail {
    OnyxGuardrail {
        api_base: base.to_owned(),
        api_key: SecretString::from(API_KEY.to_owned()),
        timeout,
        client: base_builder().build().unwrap(),
        new_id: fixed_id,
    }
}

pub(crate) fn guardrail(base: &str) -> OnyxGuardrail {
    guardrail_with_timeout(base, Duration::from_secs(5))
}

pub(crate) fn object(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn inputs(json: &str) -> BTreeMap<String, GuardrailValue> {
    object(json)
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapturedRequest {
    pub(crate) payload: GuardrailValue,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) conversation_id: String,
}

pub(crate) fn captured(mock: &MockOnyx) -> CapturedRequest {
    let seen = mock.seen.lock().unwrap();
    serde_json::from_slice(&seen.first().unwrap().body).unwrap()
}

pub(crate) fn captured_path(mock: &MockOnyx) -> String {
    let seen = mock.seen.lock().unwrap();
    seen.first().unwrap().path.clone()
}

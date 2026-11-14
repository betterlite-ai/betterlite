use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri};
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::azure_content_safety::base::{AzureGuardrailBase, DEFAULT_TIMEOUT};
use crate::azure_content_safety::legacy::{
    LegacyAzureContentSafety, LegacyAzureContentSafetyParams,
};

#[derive(Clone)]
pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) query: Option<String>,
    pub(crate) subscription_key: Option<String>,
    pub(crate) content_type: Option<String>,
    pub(crate) body: String,
}

pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) payload: String,
    pub(crate) headers: Vec<(&'static str, &'static str)>,
}

pub(crate) struct Mock {
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<io::Result<()>>,
}

pub(crate) fn reply(status: u16, payload: &str) -> Reply {
    Reply {
        status,
        payload: payload.to_owned(),
        headers: Vec::new(),
    }
}

pub(crate) async fn mock(status: u16, payload: &'static str) -> Mock {
    mock_with(vec![reply(status, payload)], None).await
}

pub(crate) async fn mock_slow(status: u16, payload: &'static str, delay: Duration) -> Mock {
    mock_with(vec![reply(status, payload)], Some(delay)).await
}

pub(crate) async fn mock_body(status: u16, payload: &str) -> Mock {
    mock_with(vec![reply(status, payload)], None).await
}

fn record(entries: &Mutex<Vec<Seen>>, uri: &Uri, headers: &HeaderMap, bytes: &Bytes) -> usize {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let mut log = entries.lock().unwrap();
    log.push(Seen {
        path: uri.path().to_owned(),
        query: uri.query().map(str::to_owned),
        subscription_key: header("ocp-apim-subscription-key"),
        content_type: header("content-type"),
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    });
    log.len().saturating_sub(1)
}

fn answer_with(chosen: &Reply) -> (StatusCode, HeaderMap, String) {
    let mut reply_headers = HeaderMap::new();
    reply_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    for &(name, value) in &chosen.headers {
        reply_headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    (
        StatusCode::from_u16(chosen.status).unwrap(),
        reply_headers,
        chosen.payload.clone(),
    )
}

// The mock answers the n-th request with the n-th reply and every later one with the last.
pub(crate) async fn mock_with(replies: Vec<Reply>, delay: Option<Duration>) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    let script = Arc::new(replies);
    let answer = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let request_log = Arc::clone(&recorder);
        let answers = Arc::clone(&script);
        async move {
            if let Some(pause) = delay {
                sleep(pause).await;
            }
            let index = record(&request_log, &uri, &headers, &bytes);
            answer_with(answers.get(index).or_else(|| answers.last()).unwrap())
        }
    };
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, Router::new().fallback(answer)).into_future());
    Mock {
        base,
        seen,
        _server: server,
    }
}

pub(crate) fn base(
    api_base: &str,
    api_version: &str,
    timeout: Option<Duration>,
) -> AzureGuardrailBase {
    AzureGuardrailBase {
        async_handler: base_builder().build().unwrap(),
        api_key: SecretString::from("azure-key".to_owned()),
        api_base: api_base.to_owned(),
        api_version: api_version.to_owned(),
        timeout: timeout.unwrap_or(DEFAULT_TIMEOUT),
    }
}

// Retries skip the SDK's 0.8s backoff here; legacy_retry's own cases check the delays.
pub(crate) fn legacy(
    endpoint: &str,
    thresholds: Option<BTreeMap<String, i64>>,
) -> LegacyAzureContentSafety {
    let mut hook = LegacyAzureContentSafety::new(
        LegacyAzureContentSafetyParams {
            endpoint: endpoint.to_owned(),
            api_key: SecretString::from("azure-key".to_owned()),
            thresholds: thresholds.unwrap_or_default(),
        },
        base_builder().build().unwrap(),
    )
    .unwrap();
    hook.retry_backoff_factor = Duration::ZERO;
    hook
}

pub(crate) fn threshold_map(entries: &[(&str, i64)]) -> BTreeMap<String, i64> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect()
}

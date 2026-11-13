use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri};
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::azure_content_safety::base::AzureGuardrailBase;
use crate::azure_content_safety::legacy::{
    LegacyAzureContentSafety, LegacyAzureContentSafetyParams,
};

pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) query: Option<String>,
    pub(crate) subscription_key: Option<String>,
    pub(crate) content_type: Option<String>,
    pub(crate) body: String,
}

pub(crate) struct Mock {
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<io::Result<()>>,
}

pub(crate) async fn mock(status: u16, payload: &'static str) -> Mock {
    mock_with(status, payload.to_owned(), None).await
}

pub(crate) async fn mock_slow(status: u16, payload: &'static str, delay: Duration) -> Mock {
    mock_with(status, payload.to_owned(), Some(delay)).await
}

pub(crate) async fn mock_body(status: u16, payload: String) -> Mock {
    mock_with(status, payload, None).await
}

async fn mock_with(status: u16, payload: String, delay: Option<Duration>) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    let answer = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let recorder = Arc::clone(&recorder);
        let reply = payload.clone();
        async move {
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
            let header = |name: &str| {
                headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned)
            };
            recorder.lock().unwrap().push(Seen {
                path: uri.path().to_owned(),
                query: uri.query().map(str::to_owned),
                subscription_key: header("ocp-apim-subscription-key"),
                content_type: header("content-type"),
                body: String::from_utf8(bytes.to_vec()).unwrap(),
            });
            (
                StatusCode::from_u16(status).unwrap(),
                [("content-type", "application/json")],
                reply,
            )
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
    api_version: Option<&str>,
    timeout: Option<Duration>,
) -> AzureGuardrailBase {
    AzureGuardrailBase {
        async_handler: base_builder().build().unwrap(),
        api_key: SecretString::from("azure-key".to_owned()),
        api_base: api_base.to_owned(),
        api_version: api_version.map(str::to_owned),
        timeout,
    }
}

pub(crate) fn legacy(
    endpoint: &str,
    thresholds: Option<BTreeMap<String, i64>>,
) -> LegacyAzureContentSafety {
    LegacyAzureContentSafety::new(
        LegacyAzureContentSafetyParams {
            endpoint: endpoint.to_owned(),
            api_key: SecretString::from("azure-key".to_owned()),
            thresholds,
        },
        base_builder().build().unwrap(),
    )
    .unwrap()
}

pub(crate) fn threshold_map(entries: &[(&str, i64)]) -> BTreeMap<String, i64> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect()
}

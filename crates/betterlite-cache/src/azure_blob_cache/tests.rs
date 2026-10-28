#![cfg(test)]

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use azure_core::credentials::{AccessToken, TokenCredential, TokenRequestOptions};
use azure_core::http::headers::{ERROR_CODE, HeaderName, Headers};
use azure_core::http::request::Body;
use azure_core::http::{AsyncRawResponse, HttpClient, Method, Request, StatusCode, Transport};
use azure_core::time::{Duration as AzureDuration, OffsetDateTime};
use betterlite_config::sections::azure_blob_cache::AzureBlobSettings;
use futures_util::future::join_all;
use tokio::runtime::Handle;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::body::decode_body;
use super::{AzureBlobCache, Disconnect as _};

#[test]
fn replays_the_azure_blob_cache_body_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/azure_blob_cache_body");
    let results: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            decode_body(&bytes)
        })
        .collect();
    assert!(!results.is_empty());
}

#[derive(Debug)]
struct FakeCredential;

#[async_trait]
impl TokenCredential for FakeCredential {
    async fn get_token(
        &self,
        _scopes: &[&str],
        _options: Option<TokenRequestOptions<'_>>,
    ) -> azure_core::Result<AccessToken> {
        Ok(AccessToken::new(
            "fake-token",
            OffsetDateTime::now_utc().saturating_add(AzureDuration::hours(1)),
        ))
    }
}

enum BlobState {
    Body(Vec<u8>),
    Error(StatusCode, &'static str),
}

impl Clone for BlobState {
    fn clone(&self) -> Self {
        match self {
            Self::Body(body) => Self::Body(body.clone()),
            Self::Error(status, code) => Self::Error(*status, code),
        }
    }
}

impl fmt::Debug for BlobState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BlobState")
    }
}

#[derive(Clone, Copy)]
enum ContainerOutcome {
    Created,
    AlreadyExists,
    Failure,
}

impl fmt::Debug for ContainerOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ContainerOutcome")
    }
}

#[derive(Debug)]
struct FakeTransport {
    container_outcome: ContainerOutcome,
    blobs: Mutex<HashMap<String, BlobState>>,
}

impl FakeTransport {
    fn new(container_outcome: ContainerOutcome) -> Arc<Self> {
        Arc::new(Self {
            container_outcome,
            blobs: Mutex::new(HashMap::new()),
        })
    }

    fn seed(&self, key: &str, state: BlobState) {
        self.blobs.lock().unwrap().insert(key.to_owned(), state);
    }

    fn container_response(&self) -> AsyncRawResponse {
        match self.container_outcome {
            ContainerOutcome::Created => {
                AsyncRawResponse::from_bytes(StatusCode::Created, Headers::new(), Vec::new())
            }
            ContainerOutcome::AlreadyExists => {
                error_response(StatusCode::Conflict, "ContainerAlreadyExists")
            }
            ContainerOutcome::Failure => {
                error_response(StatusCode::Forbidden, "AuthorizationFailure")
            }
        }
    }

    fn handle_put(&self, request: &Request) -> AsyncRawResponse {
        let key = blob_key(request);
        let if_not_exists = request
            .headers()
            .get_optional_str(&HeaderName::from_static("if-none-match"))
            .is_some();
        let mut blobs = self.blobs.lock().unwrap();
        if if_not_exists && blobs.contains_key(&key) {
            return error_response(StatusCode::Conflict, "BlobAlreadyExists");
        }
        let body = match request.body() {
            Body::Bytes(bytes) => bytes.to_vec(),
            Body::SeekableStream(_) => panic!("unexpected streaming request body"),
        };
        blobs.insert(key, BlobState::Body(body));
        drop(blobs);
        AsyncRawResponse::from_bytes(StatusCode::Created, Headers::new(), Vec::new())
    }

    fn handle_get(&self, request: &Request) -> AsyncRawResponse {
        let key = blob_key(request);
        let blobs = self.blobs.lock().unwrap();
        match blobs.get(&key) {
            Some(BlobState::Body(body)) => {
                AsyncRawResponse::from_bytes(StatusCode::Ok, Headers::new(), body.clone())
            }
            Some(BlobState::Error(status, code)) => error_response(*status, code),
            None => error_response(StatusCode::NotFound, "BlobNotFound"),
        }
    }
}

fn blob_key(request: &Request) -> String {
    request
        .url()
        .path()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn is_container_request(request: &Request) -> bool {
    request.url().query_pairs().any(|(key, _)| key == "restype")
}

fn error_response(status: StatusCode, error_code: &str) -> AsyncRawResponse {
    let mut headers = Headers::new();
    headers.insert(ERROR_CODE, error_code.to_owned());
    AsyncRawResponse::from_bytes(status, headers, Vec::new())
}

#[async_trait]
impl HttpClient for FakeTransport {
    async fn execute_request(&self, request: &Request) -> azure_core::Result<AsyncRawResponse> {
        if is_container_request(request) {
            return Ok(self.container_response());
        }
        Ok(match request.method() {
            Method::Put => self.handle_put(request),
            Method::Get => self.handle_get(request),
            Method::Delete | Method::Head | Method::Patch | Method::Post | _ => {
                panic!("unexpected method {:?}", request.method())
            }
        })
    }
}

async fn cache_with(transport: Arc<FakeTransport>, operation_timeout: Duration) -> AzureBlobCache {
    let settings = AzureBlobSettings {
        account_url: "https://fake.blob.core.windows.net".to_owned(),
        container: "responses".to_owned(),
    };
    let credential: Arc<dyn TokenCredential> = Arc::new(FakeCredential);
    AzureBlobCache::new(
        &settings,
        credential,
        Transport::new(transport),
        operation_timeout,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn new_succeeds_when_container_is_created() {
    let transport = FakeTransport::new(ContainerOutcome::Created);
    cache_with(transport, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn new_suppresses_container_already_exists() {
    let transport = FakeTransport::new(ContainerOutcome::AlreadyExists);
    cache_with(transport, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn new_propagates_other_container_errors() {
    let transport = FakeTransport::new(ContainerOutcome::Failure);
    let settings = AzureBlobSettings {
        account_url: "https://fake.blob.core.windows.net".to_owned(),
        container: "responses".to_owned(),
    };
    let credential: Arc<dyn TokenCredential> = Arc::new(FakeCredential);
    let result = AzureBlobCache::new(
        &settings,
        credential,
        Transport::new(transport),
        Duration::from_secs(5),
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn async_set_cache_overwrites_existing_blob() {
    let transport = FakeTransport::new(ContainerOutcome::Created);
    transport.seed("key", BlobState::Body(b"{\"old\":true}".to_vec()));
    let cache = cache_with(Arc::clone(&transport), Duration::from_secs(5)).await;
    let value = decode_body(br#"{"new":true}"#).unwrap();
    cache.async_set_cache("key", &value).await;
    let stored = cache.async_get_cache("key").await.unwrap();
    assert_eq!(stored, value);
}

#[tokio::test]
async fn set_cache_does_not_overwrite_existing_blob() {
    let transport = FakeTransport::new(ContainerOutcome::Created);
    let original = decode_body(br#"{"old":true}"#).unwrap();
    transport.seed(
        "key",
        BlobState::Body(serde_json::to_vec(&original).unwrap()),
    );
    let cache = cache_with(Arc::clone(&transport), Duration::from_secs(5)).await;
    let handle = Handle::current();
    let value = decode_body(br#"{"new":true}"#).unwrap();
    let cache_for_blocking = cache.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || {
        cache_for_blocking.set_cache(&handle, "key", &value);
    });
    tasks.join_next().await.unwrap().unwrap();
    let stored = cache.async_get_cache("key").await.unwrap();
    assert_eq!(stored, original);
}

#[tokio::test]
async fn async_get_cache_returns_value_on_hit() {
    let transport = FakeTransport::new(ContainerOutcome::Created);
    let cache = cache_with(Arc::clone(&transport), Duration::from_secs(5)).await;
    let value = decode_body(br#"{"ok":true}"#).unwrap();
    cache.async_set_cache("key", &value).await;
    let stored = cache.async_get_cache("key").await.unwrap();
    assert_eq!(stored, value);
}

#[tokio::test]
async fn async_get_cache_returns_none_on_miss() {
    let transport = FakeTransport::new(ContainerOutcome::Created);
    let cache = cache_with(transport, Duration::from_secs(5)).await;
    assert!(cache.async_get_cache("missing").await.is_none());
}

#[tokio::test]
async fn async_get_cache_returns_none_on_service_error() {
    let transport = FakeTransport::new(ContainerOutcome::Created);
    transport.seed(
        "key",
        BlobState::Error(StatusCode::Forbidden, "AuthorizationFailure"),
    );
    let cache = cache_with(transport, Duration::from_secs(5)).await;
    assert!(cache.async_get_cache("key").await.is_none());
}

#[tokio::test]
async fn async_get_cache_returns_none_on_decode_failure() {
    let transport = FakeTransport::new(ContainerOutcome::Created);
    transport.seed("key", BlobState::Body(b"not JSON".to_vec()));
    let cache = cache_with(transport, Duration::from_secs(5)).await;
    assert!(cache.async_get_cache("key").await.is_none());
}

#[tokio::test]
async fn pipeline_awaits_every_write() {
    let transport = FakeTransport::new(ContainerOutcome::Created);
    let cache = cache_with(Arc::clone(&transport), Duration::from_secs(5)).await;
    let entries: Vec<_> = (0_u32..20)
        .map(|index| (format!("key{index}"), decode_body(b"{}").unwrap()))
        .collect();
    timeout(
        Duration::from_secs(5),
        cache.async_set_cache_pipeline(entries),
    )
    .await
    .unwrap();
    let keys: Vec<String> = (0_u32..20).map(|index| format!("key{index}")).collect();
    let results = join_all(keys.iter().map(|key| cache.async_get_cache(key))).await;
    assert!(results.into_iter().all(|value| value.is_some()));
}

#[tokio::test]
async fn disconnect_does_not_panic() {
    let transport = FakeTransport::new(ContainerOutcome::Created);
    let cache = cache_with(transport, Duration::from_secs(5)).await;
    cache.disconnect().await;
}

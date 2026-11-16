#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::io::{self, Write};
use std::path::Path;
use std::str::from_utf8;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::gcs_response_cache::GcsResponseCacheSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use betterlite_logging::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth,
};
use betterlite_logging::gcs_mock::request::{GcsReadRequest, GcsRequest};
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::instrument::WithSubscriber as _;

use super::body::decode_body;
use super::headers::build_headers;
use super::{GcsCache, key_prefix};
use crate::s3_cache::Disconnect as _;
use crate::s3_cache::entry::CachedEntry;

const TOKEN: &str = "fixed-token";
const PROJECT_ID: &str = "fixed-project";
const SERVICE_ACCOUNT: &str = "/run/secrets/sa.json";
const STORAGE_ENDPOINT: &str = "https://storage.googleapis.com";

#[derive(Debug, thiserror::Error)]
#[error("the fixed GCS credentials were refused")]
struct FixedAuthError;

#[derive(Clone, Debug)]
struct FixedAuth {
    fail: bool,
    projects: Arc<Mutex<Vec<Option<String>>>>,
}

impl FixedAuth {
    fn new(fail: bool) -> Self {
        Self {
            fail,
            projects: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn access_token(&self, request: &EnsureAccessToken) -> Result<AccessToken, FixedAuthError> {
        self.projects
            .lock()
            .unwrap()
            .push(request.project_id.clone());
        if self.fail {
            return Err(FixedAuthError);
        }
        Ok(AccessToken {
            token: SecretString::from(TOKEN),
            project_id: PROJECT_ID.to_owned(),
        })
    }
}

#[async_trait]
impl VertexAuth for FixedAuth {
    type Error = FixedAuthError;

    async fn ensure_access_token_async(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, FixedAuthError> {
        self.access_token(&request)
    }

    fn ensure_access_token(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, FixedAuthError> {
        self.access_token(&request)
    }

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, FixedAuthError> {
        let token = request.auth_header.ok_or(FixedAuthError)?;
        Ok(TokenAndUrl {
            token,
            url: STORAGE_ENDPOINT.to_owned(),
        })
    }
}

#[derive(Clone)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut bytes = self
            .0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

async fn capture_log<F>(future: F) -> (F::Output, String)
where
    F: Future,
{
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = LogCapture(Arc::clone(&bytes));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer.clone())
        .finish();
    let output = future.with_subscriber(subscriber).await;
    let log = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    (output, log)
}

fn header_end(request: &[u8]) -> Option<usize> {
    let index = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?;
    index.checked_add(4)
}

fn content_length(head: &[u8]) -> usize {
    let text = from_utf8(head).unwrap().to_lowercase();
    text.split("\r\n")
        .find_map(|line| line.strip_prefix("content-length:"))
        .map_or(0, |value| value.trim().parse().unwrap())
}

fn request_complete(request: &[u8]) -> bool {
    let Some(end) = header_end(request) else {
        return false;
    };
    let Some(total) = end.checked_add(content_length(&request[..end])) else {
        return false;
    };
    request.len() >= total
}

// http::HeaderMap iterates in hash order, so two equal requests can put their headers in different order
fn sorted_request_lines(request: &[u8]) -> Vec<String> {
    let mut lines: Vec<String> = from_utf8(request)
        .unwrap()
        .split("\r\n")
        .map(str::to_owned)
        .collect();
    lines.sort_unstable();
    lines
}

async fn serve(listener: &TcpListener, response: &[u8]) -> Vec<u8> {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&chunk[..count]);
        if request_complete(&request) {
            break;
        }
    }
    stream.write_all(response).await.unwrap();
    request
}

async fn serve_pipeline(listener: &TcpListener, count: u32) -> Vec<Vec<u8>> {
    let mut requests = Vec::new();
    for _ in 0..count {
        requests.push(serve(listener, &response("200 OK", b"")).await);
    }
    requests
}

fn response(status: &str, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::outbound(&policy).expect("the pooled client builds")
}

fn settings(path: &str) -> GcsResponseCacheSettings {
    settings_with("responses", Some(SERVICE_ACCOUNT), path)
}

fn settings_with(
    bucket: &str,
    service_account: Option<&str>,
    path: &str,
) -> GcsResponseCacheSettings {
    GcsResponseCacheSettings {
        bucket_name: bucket.to_owned(),
        service_account_path: service_account.map(SecretString::from),
        path: path.to_owned(),
    }
}

fn environment(pairs: &[(&str, &str)]) -> Arc<dyn EnvironmentSource> {
    Arc::new(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<BTreeMap<String, String>>(),
    )
}

fn cache_at(
    base_url: &str,
    cache_settings: &GcsResponseCacheSettings,
    auth: FixedAuth,
) -> GcsCache<FixedAuth> {
    GcsCache::new(
        cache_settings,
        auth,
        proxied_client(),
        Some(base_url),
        environment(&[]),
    )
}

fn cache_address(listener: &TcpListener) -> String {
    format!("http://{}", listener.local_addr().unwrap())
}

fn cache_on(listener: &TcpListener, path: &str) -> GcsCache<FixedAuth> {
    let base_url = cache_address(listener);
    cache_at(&base_url, &settings(path), FixedAuth::new(false))
}

fn cache_on_with(
    listener: &TcpListener,
    cache_settings: &GcsResponseCacheSettings,
    auth: FixedAuth,
) -> GcsCache<FixedAuth> {
    let base_url = cache_address(listener);
    cache_at(&base_url, cache_settings, auth)
}

fn spawn_serve(
    runtime: &Runtime,
    listener: &Arc<TcpListener>,
    response: Vec<u8>,
) -> JoinHandle<Vec<u8>> {
    let shared = Arc::clone(listener);
    runtime.spawn(async move { serve(&shared, &response).await })
}

fn await_server(runtime: &Runtime, server: JoinHandle<Vec<u8>>) -> Vec<u8> {
    runtime
        .block_on(timeout(Duration::from_secs(5), server))
        .expect("the server answered before the timeout")
        .expect("the server task finished")
}

#[rstest]
#[case::absent("", "")]
#[case::root("/", "/")]
#[case::double_trailing("a//", "a/")]
#[case::nested("a/b", "a/b/")]
#[case::trailing("a/b/", "a/b/")]
fn prefix_from_gcs_path(#[case] path: &str, #[case] expected: &str) {
    assert_eq!(key_prefix(path), expected);
}

#[rstest]
#[case::slash("", "a/b", "a%2Fb")]
#[case::space("", "a b", "a%20b")]
#[case::percent("", "a%b", "a%25b")]
#[case::pre_encoded("", "%2F", "%252F")]
#[case::plus("", "a+b", "a%2Bb")]
#[case::unicode("", "héllo", "h%C3%A9llo")]
#[case::safe_marks("", "a_b.c-d~e", "a_b.c-d~e")]
#[case::prefix("cache", "response:key", "cache%2Fresponse%3Akey")]
#[tokio::test]
async fn object_name_percent_encoding(
    #[case] path: &str,
    #[case] key: &str,
    #[case] expected: &str,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on(&listener, path);
    let value = decode_body(br#"{"value":true}"#).unwrap();
    let operation = async {
        let miss = response("404 Not Found", b"");
        let (read, read_request) =
            tokio::join!(cache.async_get_cache(key), serve(&listener, &miss));
        assert!(read.is_none());
        let read_text = String::from_utf8(read_request).unwrap();
        let read_target =
            format!("GET /storage/v1/b/responses/o/{expected}?alt=media HTTP/1.1\r\n");
        assert!(read_text.starts_with(&read_target), "{read_text}");
        let stored = response("200 OK", b"");
        let ((), write_request) = tokio::join!(
            cache.async_set_cache(key, &value),
            serve(&listener, &stored)
        );
        let write_text = String::from_utf8(write_request).unwrap();
        let write_target = format!(
            "POST /upload/storage/v1/b/responses/o?uploadType=media&name={expected} HTTP/1.1\r\n"
        );
        assert!(write_text.starts_with(&write_target), "{write_text}");
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[rstest]
#[case::plain("responses")]
#[case::folder("my-bucket/folder")]
#[tokio::test]
async fn write_url_and_body(#[case] bucket: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on_with(
        &listener,
        &settings_with(bucket, Some(SERVICE_ACCOUNT), "cache"),
        FixedAuth::new(false),
    );
    let value = decode_body(br#"{"value":true}"#).unwrap();
    let operation = async {
        let stored = response("200 OK", b"");
        let ((), request) = tokio::join!(
            cache.async_set_cache("response:key", &value),
            serve(&listener, &stored)
        );
        let text = String::from_utf8(request).unwrap();
        let target = format!(
            "POST /upload/storage/v1/b/{bucket}/o?uploadType=media&name=cache%2Fresponse%3Akey HTTP/1.1\r\n"
        );
        assert!(text.starts_with(&target), "{text}");
        assert!(
            text.to_lowercase()
                .contains("content-type: application/json\r\n"),
            "{text}"
        );
        assert!(!text.to_lowercase().contains("cache-control"), "{text}");
        assert!(text.ends_with(r#"{"value":true}"#), "{text}");
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[rstest]
#[case::forbidden("403 Forbidden")]
#[case::server_error("500 Internal Server Error")]
#[case::redirect("302 Found")]
#[tokio::test]
async fn write_ignores_failure_status(#[case] status: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on(&listener, "");
    let value = decode_body(br#"{"value":true}"#).unwrap();
    let reply = response(status, b"<Error/>");
    let ((_, log), request) = timeout(Duration::from_secs(5), async {
        tokio::join!(
            capture_log(cache.async_set_cache("key", &value)),
            serve(&listener, &reply)
        )
    })
    .await
    .unwrap();
    assert!(!request.is_empty());
    assert!(!log.contains("GCS cache write failed"), "{log}");
}

#[tokio::test]
async fn write_swallows_transport_error() {
    let cache = cache_at("http://127.0.0.1:0", &settings(""), FixedAuth::new(false));
    let value = decode_body(br#"{"value":true}"#).unwrap();
    let ((), log) = timeout(
        Duration::from_secs(5),
        capture_log(cache.async_set_cache("key", &value)),
    )
    .await
    .unwrap();
    assert!(log.contains("DEBUG"), "{log}");
    assert!(log.contains("GCS cache write failed"), "{log}");
    assert!(!log.contains(SERVICE_ACCOUNT), "{log}");
    assert!(!log.contains(TOKEN), "{log}");
}

#[tokio::test]
async fn write_swallows_header_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on_with(&listener, &settings(""), FixedAuth::new(true));
    let value = decode_body(br#"{"value":true}"#).unwrap();
    let ((), log) = timeout(
        Duration::from_secs(5),
        capture_log(cache.async_set_cache("key", &value)),
    )
    .await
    .unwrap();
    assert!(log.contains("DEBUG"), "{log}");
    assert!(log.contains("GCS cache write failed"), "{log}");
    assert!(!log.contains(SERVICE_ACCOUNT), "{log}");
    assert!(!log.contains(TOKEN), "{log}");
    timeout(Duration::from_millis(50), listener.accept())
        .await
        .unwrap_err();
}

#[tokio::test]
async fn read_hit_on_200() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on(&listener, "cache");
    let body = br#"{"value":true}"#;
    let operation = async {
        let reply = response("200 OK", body);
        let (value, request) = tokio::join!(
            cache.async_get_cache("response:key"),
            serve(&listener, &reply)
        );
        let text = String::from_utf8(request).unwrap();
        assert!(
            text.starts_with(
                "GET /storage/v1/b/responses/o/cache%2Fresponse%3Akey?alt=media HTTP/1.1\r\n"
            ),
            "{text}"
        );
        assert_eq!(value, Some(decode_body(body).unwrap()));
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[rstest]
#[case::not_found("404 Not Found")]
#[case::unauthorized("401 Unauthorized")]
#[case::redirect("302 Found")]
#[tokio::test]
async fn read_miss_on_404_without_log(#[case] status: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on(&listener, "");
    let operation = async {
        let reply = response(status, b"<Error/>");
        let (value, request) = tokio::join!(
            capture_log(cache.async_get_cache("key")),
            serve(&listener, &reply)
        );
        assert!(!request.is_empty());
        value
    };
    let (value, log) = timeout(Duration::from_secs(5), operation).await.unwrap();
    assert!(value.is_none());
    assert!(!log.contains("GCS cache read failed"), "{log}");
}

#[rstest]
#[case::not_json(b"not json")]
#[case::invalid_utf8(b"\xff")]
#[case::nan_token(b"{\"value\": NaN}")]
#[tokio::test]
async fn read_miss_on_bad_json_logs_error(#[case] body: &[u8]) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on(&listener, "");
    let operation = async {
        let reply = response("200 OK", body);
        let (value, _) = tokio::join!(
            capture_log(cache.async_get_cache("key")),
            serve(&listener, &reply)
        );
        value
    };
    let (value, log) = timeout(Duration::from_secs(5), operation).await.unwrap();
    assert!(value.is_none());
    assert!(log.contains("ERROR"), "{log}");
    assert!(log.contains("GCS cache read failed"), "{log}");
    assert!(!log.contains("not json"), "{log}");
}

#[tokio::test]
async fn read_miss_on_header_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on_with(&listener, &settings(""), FixedAuth::new(true));
    let (value, log) = timeout(
        Duration::from_secs(5),
        capture_log(cache.async_get_cache("key")),
    )
    .await
    .unwrap();
    assert!(value.is_none());
    assert!(log.contains("ERROR"), "{log}");
    assert!(log.contains("GCS cache read failed"), "{log}");
    assert!(!log.contains(SERVICE_ACCOUNT), "{log}");
    assert!(!log.contains(TOKEN), "{log}");
    timeout(Duration::from_millis(50), listener.accept())
        .await
        .unwrap_err();
}

#[tokio::test]
async fn read_swallows_transport_error() {
    let cache = cache_at("http://127.0.0.1:0", &settings(""), FixedAuth::new(false));
    let (value, log) = timeout(
        Duration::from_secs(5),
        capture_log(cache.async_get_cache("key")),
    )
    .await
    .unwrap();
    assert!(value.is_none());
    assert!(log.contains("ERROR"), "{log}");
    assert!(log.contains("GCS cache read failed"), "{log}");
    assert!(!log.contains(SERVICE_ACCOUNT), "{log}");
    assert!(!log.contains(TOKEN), "{log}");
}

#[tokio::test]
async fn headers_sent_on_read_and_write() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on(&listener, "");
    let value = decode_body(br#"{"value":true}"#).unwrap();
    let operation = async {
        let write_reply = response("200 OK", b"");
        let ((), write_bytes) = tokio::join!(
            cache.async_set_cache("key", &value),
            serve(&listener, &write_reply)
        );
        let read_reply = response("200 OK", br#"{"value":true}"#);
        let (read, get_bytes) =
            tokio::join!(cache.async_get_cache("key"), serve(&listener, &read_reply));
        assert!(read.is_some());
        let write = String::from_utf8(write_bytes).unwrap().to_lowercase();
        let get = String::from_utf8(get_bytes).unwrap().to_lowercase();
        let bearer = format!("authorization: bearer {TOKEN}\r\n");
        assert!(write.contains(&bearer), "{write}");
        assert!(get.contains(&bearer), "{get}");
        assert!(
            write.contains("content-type: application/json\r\n"),
            "{write}"
        );
        assert!(get.contains("content-type: application/json\r\n"), "{get}");
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[test]
fn async_matches_sync() {
    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let handle = runtime.handle().clone();
    let listener = Arc::new(runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap());
    let cache = cache_on(&listener, "cache");
    let value = decode_body(br#"{"value":true}"#).unwrap();
    let write_server = spawn_serve(&runtime, &listener, response("200 OK", b""));
    cache.set_cache(&handle, "key", &value);
    let sync_write = await_server(&runtime, write_server);
    let async_write_server = spawn_serve(&runtime, &listener, response("200 OK", b""));
    runtime.block_on(cache.async_set_cache("key", &value));
    let async_write = await_server(&runtime, async_write_server);
    assert_eq!(
        sorted_request_lines(&sync_write),
        sorted_request_lines(&async_write)
    );
    let body = br#"{"value":true}"#;
    let read_server = spawn_serve(&runtime, &listener, response("200 OK", body));
    let sync_read = cache.get_cache(&handle, "key");
    let sync_get = await_server(&runtime, read_server);
    let async_read_server = spawn_serve(&runtime, &listener, response("200 OK", body));
    let async_read = runtime.block_on(cache.async_get_cache("key"));
    let async_get = await_server(&runtime, async_read_server);
    assert_eq!(sync_read, async_read);
    assert_eq!(sync_read, Some(value));
    assert_eq!(
        sorted_request_lines(&sync_get),
        sorted_request_lines(&async_get)
    );
    runtime.block_on(cache.disconnect());
}

#[tokio::test]
async fn sync_wrappers_refuse_inside_runtime() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on(&listener, "");
    let value = decode_body(br#"{"value":true}"#).unwrap();
    let handle = Handle::current();
    let (_, log) = capture_log(async {
        cache.set_cache(&handle, "key", &value);
        assert!(cache.get_cache(&handle, "key").is_none());
    })
    .await;
    assert!(log.contains("GCS cache write failed"), "{log}");
    assert!(log.contains("GCS cache read failed"), "{log}");
    assert!(
        log.to_lowercase()
            .contains("sync cache called inside async runtime"),
        "{log}"
    );
    timeout(Duration::from_millis(50), listener.accept())
        .await
        .unwrap_err();
}

#[tokio::test]
async fn pipeline_writes_every_pair() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on(&listener, "");
    let entries: Vec<(String, CachedEntry)> = (0_u32..20)
        .map(|index| {
            (
                format!("key{index}"),
                decode_body(br#"{"value":true}"#).unwrap(),
            )
        })
        .collect();
    let operation = async {
        let ((), requests) = tokio::join!(
            cache.async_set_cache_pipeline(entries),
            serve_pipeline(&listener, 20)
        );
        assert_eq!(requests.len(), 20);
        let recorded: Vec<String> = requests
            .into_iter()
            .map(|bytes| String::from_utf8(bytes).unwrap())
            .collect();
        let matched = (0_u32..20)
            .map(|index| {
                format!(
                    "POST /upload/storage/v1/b/responses/o?uploadType=media&name=key{index} HTTP/1.1\r\n"
                )
            })
            .all(|target| recorded.iter().any(|text| text.starts_with(target.as_str())));
        assert!(matched, "every pipeline write must appear");
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[test]
fn sync_hit_logs_key_at_debug() {
    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let handle = runtime.handle().clone();
    let listener = Arc::new(runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap());
    let cache = cache_on(&listener, "");
    let server = spawn_serve(
        &runtime,
        &listener,
        response("200 OK", br#"{"value":true}"#),
    );
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = LogCapture(Arc::clone(&bytes));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer.clone())
        .finish();
    let value = tracing::subscriber::with_default(subscriber, || cache.get_cache(&handle, "key"));
    let request = await_server(&runtime, server);
    assert!(!request.is_empty());
    assert!(value.is_some());
    let log = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(log.contains("DEBUG GCS cache hit"), "{log}");
    assert!(log.contains("key=key"), "{log}");
    assert!(!log.contains(r#"{"value":true}"#), "{log}");
}

#[tokio::test]
async fn async_hit_is_silent() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on(&listener, "");
    let operation = async {
        let reply = response("200 OK", br#"{"value":true}"#);
        let (captured, _) = tokio::join!(
            capture_log(cache.async_get_cache("key")),
            serve(&listener, &reply)
        );
        captured
    };
    let (value, log) = timeout(Duration::from_secs(5), operation).await.unwrap();
    assert!(value.is_some());
    assert!(!log.contains("GCS cache hit"), "{log}");
    assert!(!log.contains("GCS cache read failed"), "{log}");
}

#[test]
fn default_base_url_is_storage_googleapis() {
    let cache = GcsCache::new(
        &settings(""),
        FixedAuth::new(false),
        proxied_client(),
        None,
        environment(&[]),
    );
    let debug = format!("{cache:?}");
    assert!(debug.contains("https://storage.googleapis.com"), "{debug}");
}

#[test]
fn mock_environment_is_ignored() {
    let cache = GcsCache::new(
        &settings(""),
        FixedAuth::new(false),
        proxied_client(),
        None,
        environment(&[("GCS_MOCK", "true"), ("GCS_MOCK_LATENCY_MS", "1")]),
    );
    assert!(!cache.base.is_mock_mode);
    assert!(!cache.base.client.settings.enabled);
}

#[tokio::test]
async fn base_url_trailing_slash_is_trimmed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("{}/", cache_address(&listener));
    let cache = cache_at(&base_url, &settings(""), FixedAuth::new(false));
    let value = decode_body(br#"{"value":true}"#).unwrap();
    let operation = async {
        let stored = response("200 OK", b"");
        let ((), request) = tokio::join!(
            cache.async_set_cache("key", &value),
            serve(&listener, &stored)
        );
        assert!(
            String::from_utf8(request)
                .unwrap()
                .starts_with("POST /upload/")
        );
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[tokio::test]
async fn credentials_absent_from_debug_output() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on(&listener, "");
    let debug = format!("{cache:?}");
    assert!(debug.contains("responses"), "{debug}");
    assert!(!debug.contains(SERVICE_ACCOUNT), "{debug}");
    assert!(!debug.contains(TOKEN), "{debug}");
    let (value, log) = timeout(
        Duration::from_secs(5),
        capture_log(async {
            let reply = response("200 OK", b"not json");
            let (value, _) = tokio::join!(cache.async_get_cache("key"), serve(&listener, &reply));
            value
        }),
    )
    .await
    .unwrap();
    assert!(value.is_none());
    assert!(log.contains("GCS cache read failed"), "{log}");
    assert!(!log.contains(SERVICE_ACCOUNT), "{log}");
    assert!(!log.contains(TOKEN), "{log}");
}

#[tokio::test]
async fn project_id_comes_from_the_injected_environment() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let auth = FixedAuth::new(false);
    let cache = GcsCache::new(
        &settings(""),
        auth.clone(),
        proxied_client(),
        Some(cache_address(&listener).as_str()),
        environment(&[("GOOGLE_SECRET_MANAGER_PROJECT_ID", "injected-project")]),
    );
    let operation = async {
        let miss = response("404 Not Found", b"");
        let (value, _) = tokio::join!(cache.async_get_cache("key"), serve(&listener, &miss));
        assert!(value.is_none());
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
    assert_eq!(
        *auth.projects.lock().unwrap(),
        [Some("injected-project".to_owned())]
    );
}

#[tokio::test]
async fn project_id_is_absent_without_the_environment_variable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let auth = FixedAuth::new(false);
    let cache = GcsCache::new(
        &settings(""),
        auth.clone(),
        proxied_client(),
        Some(cache_address(&listener).as_str()),
        environment(&[]),
    );
    let operation = async {
        let miss = response("404 Not Found", b"");
        let (value, _) = tokio::join!(cache.async_get_cache("key"), serve(&listener, &miss));
        assert!(value.is_none());
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
    assert_eq!(*auth.projects.lock().unwrap(), [None::<String>]);
}

#[tokio::test]
async fn built_headers_hide_the_token_from_debug() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = cache_on(&listener, "");
    let headers = build_headers(
        &cache.base,
        cache.bucket_name.clone(),
        cache.service_account_path.clone(),
    )
    .await
    .unwrap();
    let request = GcsRequest::Get(GcsReadRequest {
        url: "https://storage.googleapis.com/storage/v1/b/responses/o/key".to_owned(),
        params: BTreeMap::new(),
        headers: headers.clone(),
        follow_redirects: None,
    });
    let debug = format!("{request:?}");
    assert!(debug.contains("Sensitive"), "{debug}");
    assert!(!debug.contains(TOKEN), "{debug}");
    assert!(headers.get("authorization").unwrap().is_sensitive());
    assert_eq!(
        headers.get("authorization").unwrap().to_str().unwrap(),
        format!("Bearer {TOKEN}")
    );
}

#[test]
fn replays_the_gcs_cache_body_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/gcs_cache_body");
    let results: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            decode_body(&bytes)
        })
        .collect();
    assert!(!results.is_empty());
}

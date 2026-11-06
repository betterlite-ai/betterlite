use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_config::sections::huggingface_hub::{OfflineSetting, Settings as HubSettings};
use secrecy::SecretString;
use tokio::fs;

use crate::engine::hub::{Clock, HubDownloader, HubError, SystemClock};
use crate::engine::tests::support::{Reply, Request, Server, hub_settings, resolver};

const BODY: &[u8] = b"{\"tokenizer\":\"json\"}";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const ETAG: &str = "etag-value";
const REPO: &str = "owner/name";

fn clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock)
}

fn downloader(settings: &HubSettings) -> HubDownloader {
    HubDownloader::new(
        Arc::new(settings.clone()),
        resolver(settings),
        Arc::new(SystemClock),
    )
}

fn standard_reply(request: &Request, body: &[u8]) -> Reply {
    if request.method == "HEAD" {
        let quoted = format!("\"{ETAG}\"");
        if request.header("if-none-match") == Some(quoted.as_str()) {
            return Reply::Response {
                status: 304,
                headers: vec![("ETag", quoted), ("X-Repo-Commit", COMMIT.to_owned())],
                body: Vec::new(),
            };
        }
        return Reply::Response {
            status: 200,
            headers: vec![
                ("ETag", quoted),
                ("X-Repo-Commit", COMMIT.to_owned()),
                ("Content-Length", body.len().to_string()),
            ],
            body: Vec::new(),
        };
    }
    Reply::Response {
        status: 200,
        headers: Vec::new(),
        body: body.to_vec(),
    }
}

fn cache_path(directory: &Path) -> PathBuf {
    directory
        .join("models--owner--name")
        .join("snapshots")
        .join(COMMIT)
        .join("tokenizer.json")
}

#[tokio::test]
async fn fresh_download_and_cached_etag() {
    let server = Server::start(|request| standard_reply(request, BODY)).await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    let hub = downloader(&settings);
    assert_eq!(hub.download(REPO, "main", None).await.unwrap(), BODY);
    assert_eq!(server.request_count(), 2);
    let snapshot = cache_path(directory.path());
    assert_eq!(fs::read(&snapshot).await.unwrap(), BODY);
    let reference = directory
        .path()
        .join("models--owner--name")
        .join("refs")
        .join("main");
    assert_eq!(fs::read_to_string(&reference).await.unwrap().trim(), COMMIT);
    assert_eq!(hub.download(REPO, "main", None).await.unwrap(), BODY);
    assert_eq!(server.request_count(), 3);
    let methods: Vec<String> = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.method.clone())
        .collect();
    assert_eq!(methods, ["HEAD", "GET", "HEAD"]);
}

#[tokio::test]
async fn offline_answers_from_the_cache_only() {
    let server = Server::start(|request| standard_reply(request, BODY)).await;
    let directory = tempfile::tempdir().unwrap();
    let online = hub_settings(&server.endpoint, directory.path());
    assert_eq!(
        downloader(&online)
            .download(REPO, "main", None)
            .await
            .unwrap(),
        BODY
    );
    let requests = server.request_count();
    let mut offline = hub_settings(&server.endpoint, directory.path());
    offline.offline = OfflineSetting::Offline;
    assert_eq!(
        downloader(&offline)
            .download(REPO, "main", None)
            .await
            .unwrap(),
        BODY
    );
    assert_eq!(server.request_count(), requests);
    let missing = tempfile::tempdir().unwrap();
    let empty = hub_settings(&server.endpoint, missing.path());
    let mut empty_offline = empty;
    empty_offline.offline = OfflineSetting::Offline;
    assert!(matches!(
        downloader(&empty_offline)
            .download(REPO, "main", None)
            .await,
        Err(HubError::CacheMiss { .. })
    ));
}

#[tokio::test]
async fn unauthorized_is_reported() {
    let server = Server::start(|_request| Reply::Response {
        status: 401,
        headers: Vec::new(),
        body: Vec::new(),
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    assert!(matches!(
        downloader(&settings).download(REPO, "main", None).await,
        Err(HubError::Unauthorized { .. })
    ));
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn missing_revision_and_file_do_not_fall_back() {
    let server = Server::start(|_request| Reply::Response {
        status: 404,
        headers: vec![("X-Error-Code", "RevisionNotFound".to_owned())],
        body: Vec::new(),
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    let hub = downloader(&settings);
    assert!(matches!(
        hub.download(REPO, "missing-revision", None).await,
        Err(HubError::EntryNotFound { .. })
    ));
    assert_eq!(server.request_count(), 1);
    assert!(matches!(
        hub.download(REPO, "main", None).await,
        Err(HubError::EntryNotFound { .. })
    ));
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn service_unavailable_answers_from_the_cache() {
    let server = Server::start(|request| standard_reply(request, BODY)).await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    assert_eq!(
        downloader(&settings)
            .download(REPO, "main", None)
            .await
            .unwrap(),
        BODY
    );
    let failing = Server::start(|_request| Reply::Response {
        status: 503,
        headers: Vec::new(),
        body: Vec::new(),
    })
    .await;
    let failing_settings = hub_settings(&failing.endpoint, directory.path());
    assert_eq!(
        downloader(&failing_settings)
            .download(REPO, "main", None)
            .await
            .unwrap(),
        BODY
    );
}

#[tokio::test(start_paused = true)]
async fn head_past_etag_timeout_falls_back_and_retries() {
    let server = Server::start(|_request| Reply::Stall).await;
    let directory = tempfile::tempdir().unwrap();
    let mut settings = hub_settings(&server.endpoint, directory.path());
    settings.etag_timeout = 0;
    assert!(matches!(
        downloader(&settings).download(REPO, "main", None).await,
        Err(HubError::NotAvailable { .. })
    ));
}

#[tokio::test]
async fn retry_after_cache_miss_succeeds() {
    let counter = Arc::new(AtomicUsize::new(0));
    let handler_counter = Arc::clone(&counter);
    let server = Server::start(move |request| {
        let index = handler_counter.fetch_add(1, Ordering::SeqCst);
        if index < 6 {
            return Reply::Response {
                status: 503,
                headers: Vec::new(),
                body: Vec::new(),
            };
        }
        standard_reply(request, BODY)
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let mut settings = hub_settings(&server.endpoint, directory.path());
    settings.etag_timeout = 30;
    let hub = downloader(&settings);
    assert_eq!(hub.download(REPO, "main", None).await.unwrap(), BODY);
    assert_eq!(server.request_count(), 8);
}

#[tokio::test]
async fn stalled_body_fails_with_the_download_timeout() {
    let server = Server::start(|request| {
        if request.method == "HEAD" {
            return standard_reply(request, BODY);
        }
        Reply::HeadersThenStall {
            headers: vec![("Content-Length", "1024".to_owned())],
        }
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let mut settings = hub_settings(&server.endpoint, directory.path());
    settings.download_timeout = 0;
    assert!(matches!(
        downloader(&settings).download(REPO, "main", None).await,
        Err(HubError::Stalled { .. })
    ));
}

#[tokio::test]
async fn token_header_is_present_and_absent() {
    let server = Server::start(|request| standard_reply(request, BODY)).await;
    let directory = tempfile::tempdir().unwrap();
    let mut settings = hub_settings(&server.endpoint, directory.path());
    settings.env_token = Some(SecretString::from("hf_test_token"));
    assert_eq!(
        downloader(&settings)
            .download(REPO, "main", None)
            .await
            .unwrap(),
        BODY
    );
    let authorization = server
        .requests
        .lock()
        .unwrap()
        .first()
        .and_then(|request| request.header("authorization").map(str::to_owned));
    assert_eq!(authorization.as_deref(), Some("Bearer hf_test_token"));
    let anonymous = Server::start(|request| standard_reply(request, BODY)).await;
    let anonymous_directory = tempfile::tempdir().unwrap();
    let anonymous_settings = hub_settings(&anonymous.endpoint, anonymous_directory.path());
    assert_eq!(
        downloader(&anonymous_settings)
            .download(REPO, "main", None)
            .await
            .unwrap(),
        BODY
    );
    assert!(
        anonymous
            .requests
            .lock()
            .unwrap()
            .first()
            .is_some_and(|request| request.header("authorization").is_none())
    );
}

#[tokio::test]
async fn custom_tokenizer_uses_its_explicit_token() {
    let server = Server::start(|request| standard_reply(request, BODY)).await;
    let directory = tempfile::tempdir().unwrap();
    let mut settings = hub_settings(&server.endpoint, directory.path());
    settings.env_token = Some(SecretString::from("implicit-token"));
    let explicit = SecretString::from("explicit-token");
    let hub = downloader(&settings);
    assert_eq!(
        hub.download("owner/name", "rev-9", Some(&explicit))
            .await
            .unwrap(),
        BODY
    );
    let requests = server.requests.lock().unwrap();
    let request = requests.first().unwrap();
    let authorization = request.header("authorization").map(str::to_owned);
    let path = request.path.clone();
    drop(requests);
    assert_eq!(authorization.as_deref(), Some("Bearer explicit-token"));
    assert!(path.contains("/owner/name/resolve/rev-9/tokenizer.json"));
}

#[test]
fn system_clock_runs() {
    let clock = clock();
    let first = clock.now();
    assert!(clock.now() >= first);
}

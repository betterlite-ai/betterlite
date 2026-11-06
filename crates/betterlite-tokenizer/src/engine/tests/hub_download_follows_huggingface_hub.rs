use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use secrecy::SecretString;
use temp_env::async_with_vars;
use tokio::fs;
use tokio::time::Instant;
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Metadata};

use crate::engine::hub::{Clock, HubDownloader, HubError};
use crate::engine::tests::support::{Reply, Request, Server, hub_settings, resolver, system_clock};

const BODY: &[u8] = b"{\"tokenizer\":\"json\"}";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const ETAG: &str = "etag-value";
const REPO: &str = "owner/name";

fn clock() -> Arc<Clock> {
    system_clock()
}

fn downloader(settings: &HubSettings) -> HubDownloader {
    HubDownloader::new(
        Arc::new(settings.clone()),
        resolver(settings),
        system_clock(),
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
        Err(HubError::CacheMiss(..))
    ));
}

#[tokio::test]
async fn offline_negative_cache_hit_makes_no_request() {
    let server = Server::start(|_request| Reply::Response {
        status: 404,
        headers: vec![("X-Repo-Commit", COMMIT.to_owned())],
        body: Vec::new(),
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let online = hub_settings(&server.endpoint, directory.path());
    assert!(matches!(
        downloader(&online).download(REPO, "main", None).await,
        Err(HubError::RepoNotFound(..))
    ));
    let requests = server.request_count();
    let mut offline = hub_settings(&server.endpoint, directory.path());
    offline.offline = OfflineSetting::Offline;
    assert!(matches!(
        downloader(&offline).download(REPO, "main", None).await,
        Err(HubError::CacheMiss(..))
    ));
    assert_eq!(server.request_count(), requests);
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
        Err(HubError::Unauthorized(..))
    ));
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn missing_revision_and_file_do_not_fall_back() {
    let server = Server::start(|request| {
        let revision = request.path.starts_with("/api/models/owner/name/revision/");
        let existing = request.path == "/api/models/owner/name"
            || (revision && !request.path.ends_with("missing-revision"));
        if existing {
            return Reply::Response {
                status: 200,
                headers: Vec::new(),
                body: b"{}".to_vec(),
            };
        }
        if revision {
            return Reply::Response {
                status: 404,
                headers: Vec::new(),
                body: b"{}".to_vec(),
            };
        }
        Reply::Response {
            status: 404,
            headers: vec![("X-Error-Code", "EntryNotFound".to_owned())],
            body: Vec::new(),
        }
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    let hub = downloader(&settings);
    assert!(matches!(
        hub.download(REPO, "missing-revision", None).await,
        Err(HubError::RevisionNotFound(..))
    ));
    assert_eq!(server.request_count(), 3);
    assert!(matches!(
        hub.download(REPO, "main", None).await,
        Err(HubError::EntryNotFound(..))
    ));
    assert_eq!(server.request_count(), 6);
}

#[tokio::test]
async fn missing_repository_falls_back_to_the_cache() {
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
    let missing = Server::start(|_request| Reply::Response {
        status: 404,
        headers: vec![("X-Error-Code", "RepoNotFound".to_owned())],
        body: Vec::new(),
    })
    .await;
    let missing_settings = hub_settings(&missing.endpoint, directory.path());
    assert_eq!(
        downloader(&missing_settings)
            .download(REPO, "main", None)
            .await
            .unwrap(),
        BODY
    );
}

#[tokio::test]
async fn missing_repository_without_a_cached_file_is_reported() {
    let server = Server::start(|_request| Reply::Response {
        status: 404,
        headers: vec![("X-Error-Code", "RepoNotFound".to_owned())],
        body: Vec::new(),
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    assert!(matches!(
        downloader(&settings).download(REPO, "main", None).await,
        Err(HubError::RepoNotFound(..))
    ));
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn entry_not_found_without_the_header_checks_the_repository() {
    let repo_exists = Server::start(|request| {
        if request.path.starts_with("/api/models/") {
            return Reply::Response {
                status: 200,
                headers: Vec::new(),
                body: b"{}".to_vec(),
            };
        }
        Reply::Response {
            status: 404,
            headers: Vec::new(),
            body: Vec::new(),
        }
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&repo_exists.endpoint, directory.path());
    assert!(matches!(
        downloader(&settings).download(REPO, "main", None).await,
        Err(HubError::EntryNotFound(..))
    ));
    let repo_missing = Server::start(|_request| Reply::Response {
        status: 404,
        headers: Vec::new(),
        body: Vec::new(),
    })
    .await;
    let missing_settings = hub_settings(&repo_missing.endpoint, directory.path());
    assert!(matches!(
        downloader(&missing_settings)
            .download(REPO, "main", None)
            .await,
        Err(HubError::RepoNotFound(..))
    ));
}

#[tokio::test]
async fn gated_repository_is_reported() {
    let server = Server::start(|_request| Reply::Response {
        status: 403,
        headers: vec![("X-Error-Code", "GatedRepo".to_owned())],
        body: Vec::new(),
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    assert!(matches!(
        downloader(&settings).download(REPO, "main", None).await,
        Err(HubError::Gated(..))
    ));
    assert_eq!(server.request_count(), 1);
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
async fn head_past_etag_timeout_falls_back_to_the_cache() {
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
    let stalling = Server::start(|_request| Reply::Stall).await;
    let mut stalled_settings = hub_settings(&stalling.endpoint, directory.path());
    stalled_settings.etag_timeout = 0;
    assert_eq!(
        downloader(&stalled_settings)
            .download(REPO, "main", None)
            .await
            .unwrap(),
        BODY
    );
}

#[tokio::test(start_paused = true)]
async fn head_past_etag_timeout_retries_for_sixty_seconds() {
    let server = Server::start(|_request| Reply::Stall).await;
    let directory = tempfile::tempdir().unwrap();
    let mut settings = hub_settings(&server.endpoint, directory.path());
    settings.etag_timeout = 30;
    let started = Instant::now();
    assert!(matches!(
        downloader(&settings).download(REPO, "main", None).await,
        Err(HubError::NotAvailable(..))
    ));
    assert_eq!(server.request_count(), 2);
    assert!(started.elapsed() >= Duration::from_secs(90));
    assert!(started.elapsed() < Duration::from_secs(91));
}

#[tokio::test(start_paused = true)]
async fn stalled_repository_probe_is_bounded() {
    let server = Server::start(|request| {
        if request.path.starts_with("/api/models/") {
            return Reply::Stall;
        }
        Reply::Response {
            status: 404,
            headers: Vec::new(),
            body: Vec::new(),
        }
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let mut settings = hub_settings(&server.endpoint, directory.path());
    settings.etag_timeout = 5;
    let started = Instant::now();
    assert!(matches!(
        downloader(&settings).download(REPO, "main", None).await,
        Err(HubError::EntryNotFound(..))
    ));
    assert!(started.elapsed() < Duration::from_secs(10));
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
async fn a_connection_error_retries_once_the_hub_answers() {
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = probe.local_addr().unwrap();
    drop(probe);
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&format!("http://{address}"), directory.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let holder: Arc<Mutex<Option<Server>>> = Arc::new(Mutex::new(None));
    let counted = Arc::clone(&calls);
    let kept = Arc::clone(&holder);
    let clock: Arc<Clock> = Arc::new(move || {
        let call = counted.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        if call != 3 {
            return Instant::now();
        }
        let listener = TcpListener::bind(address).unwrap();
        let server = Server::on_listener(listener, |request| standard_reply(request, BODY));
        kept.lock().unwrap().replace(server);
        Instant::now()
    });
    let hub = HubDownloader::new(Arc::new(settings.clone()), resolver(&settings), clock);
    assert_eq!(hub.download(REPO, "main", None).await.unwrap(), BODY);
    let served = holder.lock().unwrap().take().unwrap();
    assert_eq!(served.request_count(), 2);
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
        Err(HubError::Stalled(..))
    ));
}

#[tokio::test]
async fn token_header_is_present() {
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
}

#[tokio::test]
async fn token_header_is_absent() {
    async_with_vars(
        [
            ("HF_TOKEN", None::<String>),
            ("HF_TOKEN_PATH", None::<String>),
            ("HF_HOME", None::<String>),
        ],
        async {
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
            assert!(
                server
                    .requests
                    .lock()
                    .unwrap()
                    .first()
                    .is_some_and(|request| request.header("authorization").is_none())
            );
        },
    )
    .await;
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
    let first = clock();
    assert!(clock() >= first);
}

struct CountingSubscriber(Arc<AtomicUsize>);

impl tracing::Subscriber for CountingSubscriber {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if *event.metadata().level() == Level::WARN
            && event
                .metadata()
                .target()
                .starts_with("betterlite_tokenizer")
        {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[test]
fn the_deprecated_transfer_warning_fires_once() {
    let directory = tempfile::tempdir().unwrap();
    let mut settings = hub_settings("http://127.0.0.1:1", directory.path());
    settings.transfer = TransferSetting::DeprecatedTransfer;
    let hub = downloader(&settings);
    let warnings = Arc::new(AtomicUsize::new(0));
    with_default(CountingSubscriber(Arc::clone(&warnings)), || {
        let _first = hub.client(None).unwrap();
        assert_eq!(warnings.load(Ordering::SeqCst), 1);
        let _second = hub.client(None).unwrap();
        assert_eq!(warnings.load(Ordering::SeqCst), 1);
    });
}

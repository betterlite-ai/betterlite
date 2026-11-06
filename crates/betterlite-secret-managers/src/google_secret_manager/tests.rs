#![cfg(test)]

use std::{
    collections::{BTreeMap, VecDeque},
    future::pending,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::Instant;

use super::{
    Dependencies, GoogleSecretManager, GoogleSecretManagerError, HttpResponse, NativeSecretReader,
    RequestHeaderSource, RequestHeaders, SecretTransport, cache::CacheLookup, settings::Options,
};
use crate::secret_contracts::SecretFuture;

type RecordedRequest = (String, Vec<(String, String)>);

#[derive(Default)]
struct ScriptedTransport {
    answers: Mutex<VecDeque<Result<HttpResponse, io::ErrorKind>>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl ScriptedTransport {
    fn queue(&self, status: u16, body: &str) {
        self.answers.lock().unwrap().push_back(Ok(HttpResponse {
            status,
            body: body.to_owned(),
        }));
    }

    fn queue_failure(&self) {
        self.answers
            .lock()
            .unwrap()
            .push_back(Err(io::ErrorKind::ConnectionRefused));
    }

    fn urls(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(url, _)| url.clone())
            .collect()
    }
}

impl SecretTransport for ScriptedTransport {
    fn get<'request>(
        &'request self,
        url: &'request str,
        headers: &'request RequestHeaders,
    ) -> SecretFuture<'request, HttpResponse, io::Error> {
        let exposed = headers
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        self.requests
            .lock()
            .unwrap()
            .push((url.to_owned(), exposed));
        let answer = self.answers.lock().unwrap().pop_front();
        Box::pin(async move {
            match answer {
                Some(Ok(response)) => Ok(response),
                Some(Err(kind)) => Err(io::Error::from(kind)),
                None => Err(io::Error::other("no answer queued")),
            }
        })
    }
}

#[derive(Default)]
struct CountingHeaders {
    calls: AtomicUsize,
}

impl RequestHeaderSource for CountingHeaders {
    fn sync_construct_request_headers(&self) -> Result<RequestHeaders, io::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(BTreeMap::from([
            (
                "Authorization".to_owned(),
                SecretString::from("Bearer token-1"),
            ),
            (
                "Content-Type".to_owned(),
                SecretString::from("application/json"),
            ),
        ]))
    }
}

struct NativeReader {
    answer: Option<&'static str>,
}

impl NativeSecretReader for NativeReader {
    fn sync_read_secret(&self, _secret_name: &str) -> Result<Option<SecretString>, io::Error> {
        Ok(self.answer.map(SecretString::from))
    }
}

struct Harness {
    manager: GoogleSecretManager,
    transport: Arc<ScriptedTransport>,
    headers: Arc<CountingHeaders>,
    now: Arc<Mutex<Duration>>,
}

impl Harness {
    fn advance(&self, seconds: u64) {
        let mut now = self.now.lock().unwrap();
        *now = now.saturating_add(Duration::from_secs(seconds));
    }

    async fn read(&self, name: &str) -> Result<Option<SecretString>, GoogleSecretManagerError> {
        self.manager.get_secret(name).await
    }
}

fn harness_with(options: Options, native_reader: Option<Arc<dyn NativeSecretReader>>) -> Harness {
    let transport = Arc::new(ScriptedTransport::default());
    let headers = Arc::new(CountingHeaders::default());
    let now = Arc::new(Mutex::new(Duration::ZERO));
    let clock_now = Arc::clone(&now);
    let environment = BTreeMap::from([(
        "GOOGLE_SECRET_MANAGER_PROJECT_ID".to_owned(),
        "project-one".to_owned(),
    )]);
    let transport_dependency: Arc<dyn SecretTransport> =
        Arc::<ScriptedTransport>::clone(&transport);
    let header_dependency: Arc<dyn RequestHeaderSource> = Arc::<CountingHeaders>::clone(&headers);
    let manager = GoogleSecretManager::initialize(
        &environment,
        options,
        Dependencies {
            transport: transport_dependency,
            header_source: header_dependency,
            native_reader,
            clock: Arc::new(move || *clock_now.lock().unwrap()),
        },
    )
    .unwrap();
    Harness {
        manager,
        transport,
        headers,
        now,
    }
}

fn harness() -> Harness {
    harness_with(
        Options {
            refresh_interval: Some(60),
            always_read_secret_manager: None,
            request_timeout: None,
        },
        None,
    )
}

const SECRET_BODY: &str = r#"{"name":"n","payload":{"data":"c2VjcmV0"}}"#;

#[test]
fn initialization_without_project_id_fails() {
    let failure = GoogleSecretManager::initialize(
        &BTreeMap::new(),
        Options::default(),
        Dependencies {
            transport: Arc::new(ScriptedTransport::default()),
            header_source: Arc::new(CountingHeaders::default()),
            native_reader: None,
            clock: Arc::new(|| Duration::ZERO),
        },
    );

    assert!(matches!(
        failure,
        Err(GoogleSecretManagerError::MissingProjectId)
    ));
}

#[tokio::test]
async fn fetches_the_latest_version_with_the_constructed_headers() {
    let harness = harness();
    harness.transport.queue(200, SECRET_BODY);

    let secret = harness.read("api-key").await.unwrap().unwrap();

    assert_eq!(secret.expose_secret(), "secret");
    let requests = harness.transport.requests.lock().unwrap();
    assert_eq!(
        requests[0].0,
        "https://secretmanager.googleapis.com/v1/projects/project-one/secrets/api-key/versions/latest:access"
    );
    assert_eq!(
        requests[0].1,
        vec![
            ("Authorization".to_owned(), "Bearer token-1".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
        ]
    );
}

#[rstest]
#[case::number("MQ==", "1")]
#[case::json_document("eyJhIjoxfQ==", r#"{"a":1}"#)]
#[tokio::test]
async fn cached_value_keeps_its_plaintext(#[case] encoded: &str, #[case] plaintext: &str) {
    let harness = harness();
    harness
        .transport
        .queue(200, &format!(r#"{{"payload":{{"data":"{encoded}"}}}}"#));

    harness.read("api-key").await.unwrap();
    let again = harness.read("api-key").await.unwrap().unwrap();

    assert_eq!(again.expose_secret(), plaintext);
    assert_eq!(harness.transport.urls().len(), 1);
}

#[tokio::test]
async fn cached_value_is_returned_without_a_request_or_header_construction() {
    let harness = harness();
    harness.transport.queue(200, SECRET_BODY);
    harness.read("api-key").await.unwrap();

    let again = harness.read("api-key").await.unwrap().unwrap();

    assert_eq!(again.expose_secret(), "secret");
    assert_eq!(harness.transport.urls().len(), 1);
    assert_eq!(harness.headers.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn value_is_fetched_again_after_the_refresh_interval() {
    let harness = harness();
    harness.transport.queue(200, SECRET_BODY);
    harness
        .transport
        .queue(200, r#"{"payload":{"data":"bmV3ZXI="}}"#);
    harness.read("api-key").await.unwrap();

    harness.advance(61);
    let refreshed = harness.read("api-key").await.unwrap().unwrap();

    assert_eq!(refreshed.expose_secret(), "newer");
    assert_eq!(harness.transport.urls().len(), 2);
}

#[tokio::test]
async fn failed_status_caches_absence_and_raises() {
    let harness = harness();
    harness
        .transport
        .queue(404, r#"{"error":{"message":"gone"}}"#);

    let error = harness.read("api-key").await.unwrap_err();

    assert!(matches!(
        error,
        GoogleSecretManagerError::Status { status: 404, .. }
    ));
    assert!(harness.read("api-key").await.unwrap().is_none());
    assert_eq!(harness.transport.urls().len(), 1);
}

#[rstest]
#[case::created(201)]
#[case::no_content(204)]
#[case::server_error(500)]
#[tokio::test]
async fn only_status_200_is_a_success(#[case] status: u16) {
    let harness = harness();
    harness.transport.queue(status, SECRET_BODY);

    let error = harness.read("api-key").await.unwrap_err();

    assert!(matches!(error, GoogleSecretManagerError::Status { .. }));
    assert!(harness.read("api-key").await.unwrap().is_none());
}

#[rstest]
#[case::empty_object("{}")]
#[case::empty_payload(r#"{"payload":{}}"#)]
#[case::null_data(r#"{"payload":{"data":null}}"#)]
#[case::null_payload(r#"{"payload":null}"#)]
#[tokio::test]
async fn missing_payload_data_caches_absence_and_raises(#[case] body: &str) {
    let harness = harness();
    harness.transport.queue(200, body);

    let error = harness.read("api-key").await.unwrap_err();

    assert!(matches!(
        error,
        GoogleSecretManagerError::PayloadMissing { .. }
    ));
    assert!(harness.read("api-key").await.unwrap().is_none());
    assert_eq!(harness.transport.urls().len(), 1);
}

#[tokio::test]
async fn cached_absence_is_fetched_again_once_it_expires() {
    let harness = harness();
    harness.transport.queue(404, "missing");
    harness.transport.queue(200, SECRET_BODY);
    harness.read("api-key").await.unwrap_err();
    harness.advance(60);
    assert!(harness.read("api-key").await.unwrap().is_none());

    harness.advance(1);
    let found = harness.read("api-key").await.unwrap().unwrap();

    assert_eq!(found.expose_secret(), "secret");
    assert_eq!(harness.transport.urls().len(), 2);
}

#[tokio::test]
async fn always_read_ignores_cached_values_and_cached_absence() {
    let harness = harness_with(
        Options {
            refresh_interval: Some(60),
            always_read_secret_manager: Some(true),
            request_timeout: None,
        },
        None,
    );
    harness.transport.queue(404, "missing");
    harness.transport.queue(200, SECRET_BODY);
    harness
        .transport
        .queue(200, r#"{"payload":{"data":"bmV3ZXI="}}"#);
    harness.read("api-key").await.unwrap_err();

    let first = harness.read("api-key").await.unwrap().unwrap();
    let second = harness.read("api-key").await.unwrap().unwrap();

    assert_eq!(first.expose_secret(), "secret");
    assert_eq!(second.expose_secret(), "newer");
    assert_eq!(harness.transport.urls().len(), 3);
}

#[rstest]
#[case::value(Some("native-value"))]
#[case::absent(None)]
#[tokio::test]
async fn native_reader_answer_is_used_without_cache_or_network(
    #[case] answer: Option<&'static str>,
) {
    let harness = harness_with(Options::default(), Some(Arc::new(NativeReader { answer })));

    let secret = harness.read("api-key").await.unwrap();

    assert_eq!(secret.as_ref().map(SecretString::expose_secret), answer);
    assert!(harness.transport.urls().is_empty());
    assert_eq!(harness.headers.calls.load(Ordering::SeqCst), 0);
}

#[rstest]
#[case::bad_base64(r#"{"payload":{"data":"c2Vj*"}}"#)]
#[case::bad_utf8(r#"{"payload":{"data":"/w=="}}"#)]
#[case::not_json("<html>")]
#[tokio::test]
async fn undecodable_success_is_an_error_that_is_not_cached(#[case] body: &str) {
    let harness = harness();
    harness.transport.queue(200, body);
    harness.transport.queue(200, SECRET_BODY);

    harness.read("api-key").await.unwrap_err();
    let retried = harness.read("api-key").await.unwrap().unwrap();

    assert_eq!(retried.expose_secret(), "secret");
    assert_eq!(harness.transport.urls().len(), 2);
}

#[tokio::test]
async fn transport_failure_is_an_error_that_is_not_cached() {
    let harness = harness();
    harness.transport.queue_failure();
    harness.transport.queue(200, SECRET_BODY);

    let error = harness.read("api-key").await.unwrap_err();
    let retried = harness.read("api-key").await.unwrap().unwrap();

    assert!(matches!(error, GoogleSecretManagerError::Request(_)));
    assert_eq!(retried.expose_secret(), "secret");
}

struct HangingTransport;

impl SecretTransport for HangingTransport {
    fn get<'request>(
        &'request self,
        _url: &'request str,
        _headers: &'request RequestHeaders,
    ) -> SecretFuture<'request, HttpResponse, io::Error> {
        Box::pin(pending())
    }
}

#[rstest]
#[case::default(None, 600)]
#[case::configured(Some(5), 5)]
#[tokio::test(start_paused = true)]
async fn a_hanging_request_times_out_and_caches_nothing(
    #[case] request_timeout: Option<u64>,
    #[case] expected_seconds: u64,
) {
    let manager = GoogleSecretManager::initialize(
        &BTreeMap::from([(
            "GOOGLE_SECRET_MANAGER_PROJECT_ID".to_owned(),
            "project-one".to_owned(),
        )]),
        Options {
            refresh_interval: None,
            always_read_secret_manager: None,
            request_timeout: request_timeout.map(Duration::from_secs),
        },
        Dependencies {
            transport: Arc::new(HangingTransport),
            header_source: Arc::new(CountingHeaders::default()),
            native_reader: None,
            clock: Arc::new(|| Duration::ZERO),
        },
    )
    .unwrap();
    let started = Instant::now();

    let error = manager.get_secret("api-key").await.unwrap_err();

    assert!(matches!(error, GoogleSecretManagerError::Request(_)));
    assert_eq!(started.elapsed(), Duration::from_secs(expected_seconds));
    assert!(matches!(
        manager
            .cache
            .lock()
            .unwrap()
            .lookup("api-key", Duration::ZERO),
        CacheLookup::Miss
    ));
}

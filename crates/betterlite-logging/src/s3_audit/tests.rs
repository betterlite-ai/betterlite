#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_config::loader::{ConfigLoader, EnvironmentLookup};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use chrono::{DateTime, TimeZone as _, Utc};
use indexmap::IndexMap;
use rstest::rstest;
use tokio::task::yield_now;
use tokio::time::{advance, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::dedicated::build_audit_logger;
use super::element::{AuditElementError, audit_element};
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::registry::catalog::{CatalogError, CatalogInputs};
use crate::s3_batch::flush::SystemMonotonic;
use crate::s3_batch::logger::{S3BatchDeps, S3BatchLogger};
use crate::s3_transport::batch_element::ContentType;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::options::PartitionGranularity;
use crate::s3_transport::signing::Clock;
use crate::s3_transport::transport::{S3HttpClient, S3Request, S3Response};
use crate::s3_v2::parameters::resolve_s3_parameters;
use crate::s3_v2::source::Source;

const PAYLOAD: &str = r#"{
    "id": "s3-model-event",
    "trace_id": "",
    "session_id": "",
    "call_type": "completion",
    "response_cost": 0.1,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 30,
    "prompt_tokens": 10,
    "completion_tokens": 20,
    "startTime": 1234567890.0,
    "endTime": 1234567891.0,
    "completionStartTime": 1234567890.5,
    "response_time": 0.0,
    "model_map_information": {},
    "model": "gpt-3.5-turbo",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

#[derive(Debug)]
struct Recorder {
    status: Mutex<u16>,
    sent: Mutex<Vec<S3Request>>,
}

impl Recorder {
    fn new(status: u16) -> Arc<Self> {
        Arc::new(Self {
            status: Mutex::new(status),
            sent: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|request| request.url.clone())
            .collect()
    }
}

impl S3HttpClient for Recorder {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let status = *self.status.lock().unwrap_or_else(PoisonError::into_inner);
        Box::pin(async move {
            Ok(S3Response {
                status,
                body: Vec::new(),
            })
        })
    }
}

#[derive(Debug)]
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the log is UTF-8")
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn subscribe(buffer: &Buffer, level: Level) -> DefaultGuard {
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    set_default(subscriber)
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 4, 5, 6, 7)
        .single()
        .expect("the instant is valid")
}

fn record(identifier: &str) -> Json {
    serde_json::from_str(&format!(
        r#"{{"id": "{identifier}", "action": "created", "before_value": null, "updated_values": null}}"#
    ))
    .expect("the audit record is JSON")
}

fn source(extra: &str) -> Source<()> {
    serde_json::from_str(&format!(
        r#"{{"s3_bucket_name":"logs","s3_region_name":"us-east-1","s3_endpoint_url":"http://127.0.0.1:9000"{extra}}}"#
    ))
    .expect("the S3 source loads")
}

fn logger(
    client: Arc<Recorder>,
    extra: &str,
    mut environment: BTreeMap<String, String>,
    lifecycle: Option<(&TaskTracker, &CancellationToken)>,
) -> Arc<S3BatchLogger> {
    let (tasks, shutdown) = lifecycle.map_or_else(
        || (None, CancellationToken::new()),
        |(tracker, token)| (Some(tracker.clone()), token.clone()),
    );
    environment
        .entry("AWS_ACCESS_KEY_ID".to_owned())
        .or_insert_with(|| "DEFAULTKEYID".to_owned());
    environment
        .entry("AWS_SECRET_ACCESS_KEY".to_owned())
        .or_insert_with(|| "defaultsecret".to_owned());
    let lookup: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(environment);
    let parameters =
        resolve_s3_parameters::<()>(&Source::default(), &source(extra), None, lookup.as_ref());
    S3BatchLogger::new(
        parameters,
        S3BatchDeps {
            client,
            clock: Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
            monotonic: Arc::new(SystemMonotonic::default()),
            environment: lookup,
            throttle_observer: None,
            cold_storage_logger: None,
            tasks,
            shutdown,
        },
    )
    .expect("the logger builds")
}

fn model_event(text: &str) -> Arc<LoggerEvent> {
    let payload: StandardLoggingPayload =
        serde_json::from_str(text).expect("the payload is a standard logging payload");
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

fn batch_of(size: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("DEFAULT_S3_BATCH_SIZE".to_owned(), size.to_owned())])
}

#[tokio::test(start_paused = true)]
async fn audit_record_below_batch_size_waits_for_the_timer() {
    let tracker = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let client = Recorder::new(200);
    let logger = logger(
        Arc::clone(&client),
        "",
        BTreeMap::new(),
        Some((&tracker, &shutdown)),
    );
    logger.log_audit_event(&record("a1"), now()).await;
    assert!(client.requests().is_empty());
    assert_eq!(logger.queue.snapshot().entries.len(), 1);
    advance(Duration::from_secs(10)).await;
    yield_now().await;
    let requests = client.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].ends_with("audit_logs/2026-03-04/05-06-07_a1.json"));
    assert!(logger.queue.snapshot().entries.is_empty());
    shutdown.cancel();
    tracker.close();
    timeout(Duration::from_secs(1), tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
    drop(logger);
}

#[tokio::test]
async fn audit_record_that_fills_the_batch_flushes_at_once() {
    let client = Recorder::new(200);
    let logger = logger(Arc::clone(&client), "", batch_of("2"), None);
    logger.log_audit_event(&record("a1"), now()).await;
    assert!(client.requests().is_empty());
    logger.log_audit_event(&record("a2"), now()).await;
    let requests = client.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].ends_with("audit_logs/2026-03-04/05-06-07_a1.json"));
    assert!(requests[1].ends_with("audit_logs/2026-03-04/05-06-07_a2.json"));
    assert!(logger.queue.snapshot().entries.is_empty());
    drop(logger);
}

#[tokio::test]
async fn model_event_that_fills_the_batch_does_not_flush() {
    let client = Recorder::new(200);
    let logger = logger(Arc::clone(&client), "", batch_of("1"), None);
    logger
        .async_log_success_event(model_event(PAYLOAD))
        .await
        .expect("the event is queued");
    logger
        .async_log_success_event(model_event(PAYLOAD))
        .await
        .expect("the event is queued");
    assert!(client.requests().is_empty());
    assert_eq!(logger.queue.snapshot().entries.len(), 2);
    drop(logger);
}

#[tokio::test]
async fn audit_and_model_elements_share_the_queue_in_arrival_order() {
    let client = Recorder::new(200);
    let logger = logger(Arc::clone(&client), "", BTreeMap::new(), None);
    logger
        .async_log_success_event(model_event(PAYLOAD))
        .await
        .expect("the event is queued");
    logger.log_audit_event(&record("a1"), now()).await;
    logger.log_audit_event(&record("a2"), now()).await;
    assert!(client.requests().is_empty());
    let queued = logger.queue.snapshot();
    assert_eq!(queued.entries.len(), 3);
    assert_eq!(
        queued.entries[0].s3_object_key,
        "2009-02-13/time-23-31-30-000000_s3-model-event.json"
    );
    assert_eq!(
        queued.entries[1].s3_object_key,
        "audit_logs/2026-03-04/05-06-07_a1.json"
    );
    assert_eq!(
        queued.entries[2].s3_object_key,
        "audit_logs/2026-03-04/05-06-07_a2.json"
    );
    assert_eq!(queued.entries[1].payload, record("a1"));
    assert_eq!(
        queued.entries[1].s3_object_download_filename,
        "audit-a1.json"
    );
    assert_eq!(queued.entries[1].content_type, ContentType::Json);
    assert_eq!(queued.entries[1].body, None);
    drop(logger);
}

#[tokio::test]
async fn a_record_that_is_not_an_object_logs_and_keeps_the_queue() {
    let client = Recorder::new(200);
    let logger = logger(Arc::clone(&client), "", BTreeMap::new(), None);
    let buffer = Buffer::default();
    let guard = subscribe(&buffer, Level::WARN);
    logger
        .log_audit_event(&["not", "an", "object"], now())
        .await;
    drop(guard);
    assert!(logger.queue.snapshot().entries.is_empty());
    assert!(client.requests().is_empty());
    assert!(
        buffer
            .contents()
            .contains("S3 audit log error: the S3 audit record is not a JSON object"),
        "{}",
        buffer.contents()
    );
    drop(logger);
}

#[tokio::test]
async fn a_failed_flush_logs_and_keeps_the_audit_record() {
    let client = Recorder::new(404);
    let logger = logger(Arc::clone(&client), "", batch_of("1"), None);
    let buffer = Buffer::default();
    let guard = subscribe(&buffer, Level::WARN);
    logger.log_audit_event(&record("a1"), now()).await;
    drop(guard);
    let queued = logger.queue.snapshot();
    assert_eq!(queued.entries.len(), 1);
    assert_eq!(queued.requeued_count, 1);
    assert!(
        buffer.contents().contains("S3 audit log error"),
        "{}",
        buffer.contents()
    );
    drop(logger);
}

#[tokio::test]
async fn dedicated_logger_uses_the_audit_source() {
    let inputs = inputs("", credentials());
    let audit_source: Source<()> = serde_json::from_str(
        r#"{"s3_bucket_name":"audit-bucket","s3_region_name":"us-east-1","s3_endpoint_url":"http://127.0.0.1:9000","s3_path":"audit-logs"}"#,
    )
    .expect("the audit source loads");
    let logger = build_audit_logger(&inputs, &audit_source).expect("the audit logger builds");
    assert_eq!(
        logger.parameters.connection.bucket_name.as_deref(),
        Some("audit-bucket")
    );
    assert_eq!(
        logger.parameters.connection.path.as_deref(),
        Some("audit-logs")
    );
    assert_eq!(logger.batch_size, 512);
    assert_eq!(logger.flush_interval, Duration::from_secs(10));
    logger.log_audit_event(&record("a1"), now()).await;
    let queued = logger.queue.snapshot();
    assert_eq!(queued.entries.len(), 1);
    assert_eq!(
        queued.entries[0].s3_object_key,
        "audit-logs/audit_logs/2026-03-04/05-06-07_a1.json"
    );
    assert_eq!(inputs.tracker.len(), 1);
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
    drop(logger);
}

#[tokio::test]
async fn dedicated_logger_ignores_the_global_source() {
    let inputs = inputs(
        "callback_settings:\n  s3_v2:\n    s3_bucket_name: global-bucket\n    s3_region_name: us-east-1\n    s3_path: global-logs\n",
        credentials(),
    );
    let empty = Source::<()>::default();
    let error = build_audit_logger(&inputs, &empty)
        .err()
        .expect("an empty audit source has no bucket");
    assert!(
        matches!(&error, CatalogError::Init { message } if message.contains("bucket")),
        "{error:?}"
    );
}

#[tokio::test]
async fn dedicated_logger_redacts_credentials_at_debug() {
    let inputs = inputs("", credentials());
    let audit_source: Source<()> = serde_json::from_str(
        r#"{"s3_bucket_name":"audit-bucket","s3_region_name":"us-east-1","s3_endpoint_url":"http://127.0.0.1:9000","s3_aws_access_key_id":"SOURCEKEYID","s3_aws_secret_access_key":"sourcesecret"}"#,
    )
    .expect("the audit source loads");
    let buffer = Buffer::default();
    let guard = subscribe(&buffer, Level::DEBUG);
    let logger = build_audit_logger(&inputs, &audit_source).expect("the audit logger builds");
    drop(guard);
    let logged = buffer.contents();
    assert!(
        logged.contains("in init s3 logger (audit override)"),
        "{logged}"
    );
    assert!(!logged.contains("SOURCEKEYID"), "{logged}");
    assert!(!logged.contains("sourcesecret"), "{logged}");
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
    drop(logger);
}

#[tokio::test]
async fn dedicated_logger_applies_the_audit_s3_verify() {
    let inputs = inputs("", credentials());
    let disabled: Source<()> = serde_json::from_str(
        r#"{"s3_bucket_name":"audit-bucket","s3_region_name":"us-east-1","s3_endpoint_url":"http://127.0.0.1:9000","s3_verify":false}"#,
    )
    .expect("the audit source loads");
    let logger = build_audit_logger(&inputs, &disabled).expect("a false s3_verify builds");
    let not_a_bool: Source<()> = serde_json::from_str(
        r#"{"s3_bucket_name":"audit-bucket","s3_region_name":"us-east-1","s3_endpoint_url":"http://127.0.0.1:9000","s3_verify":"maybe"}"#,
    )
    .expect("the audit source loads");
    let error = build_audit_logger(&inputs, &not_a_bool)
        .err()
        .expect("a text s3_verify is refused");
    assert!(
        matches!(
            &error,
            CatalogError::Init { message }
                if message.contains("s3_audit_callback_params.s3_verify: expected a bool, got str")
        ),
        "{error:?}"
    );
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
    drop(logger);
}

#[tokio::test]
async fn dedicated_logger_refuses_an_unsupported_security_level() {
    let environment: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(BTreeMap::from([
        ("AWS_ACCESS_KEY_ID".to_owned(), "AKIDTEST".to_owned()),
        ("AWS_SECRET_ACCESS_KEY".to_owned(), "secrettest".to_owned()),
        (
            "SSL_SECURITY_LEVEL".to_owned(),
            "DEFAULT@SECLEVEL=2".to_owned(),
        ),
    ]));
    let inputs = inputs("", environment);
    let audit_source: Source<()> = serde_json::from_str(
        r#"{"s3_bucket_name":"audit-bucket","s3_region_name":"us-east-1","s3_endpoint_url":"http://127.0.0.1:9000","s3_verify":true}"#,
    )
    .expect("the audit source loads");
    let error = build_audit_logger(&inputs, &audit_source)
        .err()
        .expect("rustls has no security level");
    assert!(
        matches!(&error, CatalogError::Init { message } if message.contains("SSL_SECURITY_LEVEL")),
        "{error:?}"
    );
}

fn credentials() -> Arc<dyn EnvironmentLookup + Send + Sync> {
    Arc::new(BTreeMap::from([
        ("AWS_ACCESS_KEY_ID".to_owned(), "AKIDTEST".to_owned()),
        ("AWS_SECRET_ACCESS_KEY".to_owned(), "secrettest".to_owned()),
    ]))
}

fn inputs(
    source_text: &str,
    environment: Arc<dyn EnvironmentLookup + Send + Sync>,
) -> CatalogInputs {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source_text)
    .expect("the config loads");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: environment,
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

#[rstest]
#[case::day_partition(
    "team-logs",
    PartitionGranularity::Day,
    r#"{"id": "a1"}"#,
    "team-logs/audit_logs/2026-03-04/05-06-07_a1.json",
    "audit-a1.json"
)]
#[case::hour_partition(
    "team-logs",
    PartitionGranularity::Hour,
    r#"{"id": "a1"}"#,
    "team-logs/audit_logs/2026-03-04/05/05-06-07_a1.json",
    "audit-a1.json"
)]
#[case::empty_path(
    "",
    PartitionGranularity::Day,
    r#"{"id": "a1"}"#,
    "audit_logs/2026-03-04/05-06-07_a1.json",
    "audit-a1.json"
)]
#[case::trailing_slash_path(
    "team-logs/",
    PartitionGranularity::Day,
    r#"{"id": "a1"}"#,
    "team-logs/audit_logs/2026-03-04/05-06-07_a1.json",
    "audit-a1.json"
)]
#[case::absent_id(
    "team-logs",
    PartitionGranularity::Day,
    r#"{"action": "created"}"#,
    "team-logs/audit_logs/2026-03-04/05-06-07_unknown.json",
    "audit-unknown.json"
)]
#[case::null_id(
    "team-logs",
    PartitionGranularity::Day,
    r#"{"id": null}"#,
    "team-logs/audit_logs/2026-03-04/05-06-07_unknown.json",
    "audit-unknown.json"
)]
#[case::slash_and_colon_in_id(
    "",
    PartitionGranularity::Day,
    r#"{"id": "key/1:x"}"#,
    "audit_logs/2026-03-04/05-06-07_key_1_x.json",
    "audit-key/1:x.json"
)]
fn audit_element_builds_the_key_and_names(
    #[case] s3_path: &str,
    #[case] granularity: PartitionGranularity,
    #[case] text: &str,
    #[case] expected_key: &str,
    #[case] expected_download: &str,
) {
    let audit_log: Json = serde_json::from_str(text).expect("the record is JSON");
    let element =
        audit_element(s3_path, granularity, &audit_log, now()).expect("the element builds");
    assert_eq!(element.s3_object_key, expected_key);
    assert_eq!(element.s3_object_download_filename, expected_download);
    assert_eq!(element.payload, audit_log);
    assert_eq!(element.body, None);
    assert_eq!(element.content_type, ContentType::Json);
    assert_eq!(element.retrying_since, None);
}

#[test]
fn audit_element_refuses_a_non_object_record() {
    let array: Json = serde_json::from_str("[1, 2]").expect("the record is JSON");
    let error = audit_element("", PartitionGranularity::Day, &array, now())
        .expect_err("a list is not an audit record");
    assert_eq!(error, AuditElementError::NotAnObject);
}

#[rstest]
#[case::number(r#"{"id": 7}"#)]
#[case::boolean(r#"{"id": true}"#)]
#[case::list(r#"{"id": ["a"]}"#)]
#[case::object(r#"{"id": {"a": 1}}"#)]
fn audit_element_refuses_a_non_text_id(#[case] text: &str) {
    let audit_log: Json = serde_json::from_str(text).expect("the record is JSON");
    let error = audit_element("", PartitionGranularity::Day, &audit_log, now())
        .expect_err("the id is not text");
    assert_eq!(error, AuditElementError::IdNotText);
}

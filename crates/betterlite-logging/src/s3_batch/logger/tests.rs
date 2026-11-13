#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use indexmap::IndexMap;
use rstest::rstest;
use tokio::time::{advance, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::callback_logger::CallbackLogger as _;
use betterlite_config::loader::EnvironmentLookup;
use tokio::task::yield_now;

use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::s3_batch::Element;
use crate::s3_batch::flush::SystemMonotonic;
use crate::s3_batch::logger::{InitError, S3BatchDeps, S3BatchLogger};
use crate::s3_transport::batch_element::ContentType;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::options::PartitionGranularity;
use crate::s3_transport::signing::Clock;
use crate::s3_transport::transport::{S3HttpClient, S3Request, S3Response};
use crate::s3_v2::parameters::resolve_s3_parameters;
use crate::s3_v2::source::Source;
use betterlite_models::decisions::Json;

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

#[derive(Debug, Default)]
struct ScriptedClient {
    replies: Mutex<VecDeque<u16>>,
    sent: Mutex<Vec<S3Request>>,
}

impl ScriptedClient {
    fn with_replies(replies: &[u16]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.iter().copied().collect()),
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

    fn header(&self, name: &str) -> Option<String> {
        let sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        let first = sent.first()?.clone();
        drop(sent);
        first
            .headers
            .into_iter()
            .find(|(key, _value)| key.eq_ignore_ascii_case(name))
            .map(|(_key, value)| value)
    }
}

impl S3HttpClient for ScriptedClient {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let status = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(200);
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

fn source(extra: &str) -> Source<()> {
    serde_json::from_str(&format!(
        r#"{{"s3_bucket_name":"logs","s3_region_name":"us-east-1","s3_endpoint_url":"http://127.0.0.1:9000"{extra}}}"#
    ))
    .expect("the S3 source loads")
}

fn logger(
    client: Arc<ScriptedClient>,
    extra: &str,
    mut environment: BTreeMap<String, String>,
    cold_storage_logger: Option<&str>,
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
            cold_storage_logger: cold_storage_logger.map(str::to_owned),
            tasks,
            shutdown,
        },
    )
    .expect("the logger builds")
}

fn event(text: &str) -> Arc<LoggerEvent> {
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

#[tokio::test]
async fn callback_logger_queues_model_events() {
    let client = ScriptedClient::with_replies(&[]);
    let logger = logger(Arc::clone(&client), "", BTreeMap::new(), None, None);
    logger
        .async_log_success_event(event(PAYLOAD))
        .await
        .expect("the event is queued");
    let sub_millisecond = PAYLOAD.replace("1234567890.0", "1234567890.123456");
    logger
        .async_log_failure_event(event(&sub_millisecond))
        .await
        .expect("the event is queued");
    assert_eq!(logger.callback_name(), "s3_v2");
    assert_eq!(logger.batch_size, 512);
    assert_eq!(logger.flush_interval, Duration::from_secs(10));
    let queued = logger.queue.snapshot();
    assert_eq!(queued.entries.len(), 2);
    assert_eq!(
        queued.entries[0].s3_object_key,
        "2009-02-13/time-23-31-30-000000_s3-model-event.json"
    );
    assert_eq!(
        queued.entries[0].s3_object_download_filename,
        "time-2009-02-13T23-31-30-000000_s3-model-event.json"
    );
    assert_eq!(
        queued.entries[1].s3_object_key,
        "2009-02-13/time-23-31-30-123456_s3-model-event.json"
    );
    assert_eq!(
        queued.entries[1].s3_object_download_filename,
        "time-2009-02-13T23-31-30-123456_s3-model-event.json"
    );
    assert_eq!(queued.entries[0].retrying_since, None);
    assert!(client.requests().is_empty());
    drop(logger);
}

#[tokio::test]
async fn callback_logger_uses_the_configured_partition_granularity() {
    let logger = logger(
        ScriptedClient::with_replies(&[]),
        "",
        BTreeMap::from([("S3_PARTITION_GRANULARITY".to_owned(), "hour".to_owned())]),
        None,
        None,
    );
    let mut cache = logger
        .partition_cache
        .lock()
        .expect("the partition cache is free");
    let resolved = cache.resolve(
        logger.parameters.connection.partition_granularity.as_ref(),
        logger.environment.as_ref(),
    );
    assert_eq!(resolved, PartitionGranularity::Hour);
    drop(cache);
    drop(logger);
}

#[tokio::test(start_paused = true)]
async fn callback_logger_flushes_on_the_interval() {
    let tracker = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let client = ScriptedClient::with_replies(&[200]);
    let logger = logger(
        Arc::clone(&client),
        "",
        BTreeMap::new(),
        None,
        Some((&tracker, &shutdown)),
    );
    logger
        .async_log_success_event(event(PAYLOAD))
        .await
        .expect("the event is queued");
    yield_now().await;
    assert!(client.requests().is_empty());
    advance(Duration::from_secs(10)).await;
    yield_now().await;
    assert_eq!(client.requests().len(), 1);
    assert!(client.requests()[0].contains("logs"));
    shutdown.cancel();
    tracker.close();
    timeout(Duration::from_secs(1), tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
    drop(logger);
}

#[tokio::test(start_paused = true)]
async fn callback_logger_stops_on_shutdown() {
    let tracker = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let client = ScriptedClient::with_replies(&[200]);
    let logger = logger(
        Arc::clone(&client),
        "",
        BTreeMap::new(),
        None,
        Some((&tracker, &shutdown)),
    );
    shutdown.cancel();
    tracker.close();
    timeout(Duration::from_secs(1), tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
    logger
        .async_log_success_event(event(PAYLOAD))
        .await
        .expect("the event is queued");
    advance(Duration::from_secs(30)).await;
    yield_now().await;
    assert!(client.requests().is_empty());
    assert_eq!(logger.queue.snapshot().entries.len(), 1);
    drop(logger);
}

#[tokio::test]
async fn callback_logger_queues_any_json_object() {
    let logger = logger(
        ScriptedClient::with_replies(&[]),
        "",
        BTreeMap::new(),
        None,
        None,
    );
    let payload: Json = serde_json::from_str(r#"{"id":"audit-1","action":"created"}"#)
        .expect("the audit record is JSON");
    logger.queue.enqueue(Element {
        payload,
        s3_object_key: "audit_logs/a.json".to_owned(),
        s3_object_download_filename: "audit-a.json".to_owned(),
        body: None,
        content_type: ContentType::Json,
        retrying_since: None,
    });
    logger
        .async_log_success_event(event(PAYLOAD))
        .await
        .expect("a model event shares the queue");
    assert_eq!(logger.queue.snapshot().entries.len(), 2);
    drop(logger);
}

#[tokio::test]
async fn callback_logger_reports_a_bad_payload_timestamp() {
    let logger = logger(
        ScriptedClient::with_replies(&[]),
        "",
        BTreeMap::new(),
        None,
        None,
    );
    let mut payload: StandardLoggingPayload =
        serde_json::from_str(PAYLOAD).expect("the payload loads");
    payload.start_time = rust_decimal::Decimal::MAX;
    let event = Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    });
    let error = logger.async_log_success_event(event).await;
    error.expect_err("a payload past the timestamp range fails");
    assert!(logger.queue.snapshot().entries.is_empty());
    drop(logger);
}

#[tokio::test]
async fn callback_logger_flush_uploads_and_empties_the_queue() {
    let client = ScriptedClient::with_replies(&[200]);
    let logger = logger(Arc::clone(&client), "", BTreeMap::new(), None, None);
    logger
        .async_log_success_event(event(PAYLOAD))
        .await
        .expect("the event is queued");
    logger.flush_queue().await.expect("the upload is delivered");
    let requests = client.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("127.0.0.1:9000"));
    assert!(requests[0].ends_with("2009-02-13/time-23-31-30-000000_s3-model-event.json"));
    assert!(logger.queue.snapshot().entries.is_empty());
    assert_eq!(logger.in_flight_flushes.load(Ordering::SeqCst), 0);
    drop(logger);
}

#[tokio::test]
async fn callback_logger_flush_keeps_a_rejected_event() {
    let client = ScriptedClient::with_replies(&[404]);
    let logger = logger(Arc::clone(&client), "", BTreeMap::new(), None, None);
    logger
        .async_log_success_event(event(PAYLOAD))
        .await
        .expect("the event is queued");
    logger
        .flush_queue()
        .await
        .expect_err("the upload is rejected");
    let kept = logger.queue.snapshot();
    assert_eq!(kept.entries.len(), 1);
    assert_eq!(kept.requeued_count, 1);
    assert_eq!(logger.in_flight_flushes.load(Ordering::SeqCst), 0);
    drop(logger);
}

#[tokio::test]
async fn callback_logger_signs_with_the_environment_credentials() {
    let client = ScriptedClient::with_replies(&[200]);
    let environment = BTreeMap::from([
        ("AWS_ACCESS_KEY_ID".to_owned(), "ENVKEYID".to_owned()),
        ("AWS_SECRET_ACCESS_KEY".to_owned(), "envsecret".to_owned()),
        ("AWS_SESSION_TOKEN".to_owned(), "envtoken".to_owned()),
    ]);
    let logger = logger(Arc::clone(&client), "", environment, None, None);
    logger
        .async_log_success_event(event(PAYLOAD))
        .await
        .expect("the event is queued");
    logger.flush_queue().await.expect("the upload is delivered");
    let authorization = client
        .header("authorization")
        .expect("the request is signed");
    assert!(authorization.contains("Credential=ENVKEYID/"));
    assert_eq!(
        client.header("x-amz-security-token").as_deref(),
        Some("envtoken")
    );
    drop(logger);
}

#[tokio::test]
async fn callback_logger_prefers_the_configured_credentials() {
    let client = ScriptedClient::with_replies(&[200]);
    let environment = BTreeMap::from([
        ("AWS_ACCESS_KEY_ID".to_owned(), "ENVKEYID".to_owned()),
        ("AWS_SECRET_ACCESS_KEY".to_owned(), "envsecret".to_owned()),
        ("AWS_SESSION_TOKEN".to_owned(), "envtoken".to_owned()),
    ]);
    let logger = logger(
        Arc::clone(&client),
        r#","s3_aws_access_key_id":"SRCKEYID","s3_aws_secret_access_key":"srcsecret","s3_aws_session_token":"srctoken""#,
        environment,
        None,
        None,
    );
    logger
        .async_log_success_event(event(PAYLOAD))
        .await
        .expect("the event is queued");
    logger.flush_queue().await.expect("the upload is delivered");
    let authorization = client
        .header("authorization")
        .expect("the request is signed");
    assert!(authorization.contains("Credential=SRCKEYID/"));
    assert_eq!(
        client.header("x-amz-security-token").as_deref(),
        Some("srctoken")
    );
    drop(logger);
}

#[rstest]
#[case::empty_key_variable("", Some(""), Some("envsecret"))]
#[case::no_key("", None, Some("envsecret"))]
#[case::no_secret(r#","s3_aws_access_key_id":"SRCKEYID""#, None, None)]
#[tokio::test]
async fn callback_logger_refuses_missing_credentials(
    #[case] extra: &str,
    #[case] key_id: Option<&str>,
    #[case] secret: Option<&str>,
) {
    let variables = [
        ("AWS_ACCESS_KEY_ID", key_id),
        ("AWS_SECRET_ACCESS_KEY", secret),
    ];
    let environment: BTreeMap<String, String> = variables
        .into_iter()
        .filter_map(|(name, value)| Some((name.to_owned(), value?.to_owned())))
        .collect();
    let lookup: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(environment);
    let parameters =
        resolve_s3_parameters::<()>(&Source::default(), &source(extra), None, lookup.as_ref());
    let built = S3BatchLogger::new(
        parameters,
        S3BatchDeps {
            client: ScriptedClient::with_replies(&[]),
            clock: Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
            monotonic: Arc::new(SystemMonotonic::default()),
            environment: lookup,
            throttle_observer: None,
            cold_storage_logger: None,
            tasks: None,
            shutdown: CancellationToken::new(),
        },
    );
    assert!(matches!(built, Err(InitError::MissingCredentials)));
}

#[tokio::test]
async fn callback_logger_reads_the_batch_schedule_from_the_environment() {
    let tuned = logger(
        ScriptedClient::with_replies(&[]),
        "",
        BTreeMap::from([
            ("DEFAULT_S3_BATCH_SIZE".to_owned(), " 7 ".to_owned()),
            (
                "DEFAULT_S3_FLUSH_INTERVAL_SECONDS".to_owned(),
                "3".to_owned(),
            ),
        ]),
        None,
        None,
    );
    assert_eq!(tuned.batch_size, 7);
    assert_eq!(tuned.flush_interval, Duration::from_secs(3));
    let invalid = logger(
        ScriptedClient::with_replies(&[]),
        "",
        BTreeMap::from([
            ("DEFAULT_S3_BATCH_SIZE".to_owned(), "0".to_owned()),
            (
                "DEFAULT_S3_FLUSH_INTERVAL_SECONDS".to_owned(),
                "soon".to_owned(),
            ),
        ]),
        None,
        None,
    );
    assert_eq!(invalid.batch_size, 512);
    assert_eq!(invalid.flush_interval, Duration::from_secs(10));
    drop((tuned, invalid));
}

#[tokio::test]
async fn callback_logger_adaptive_throttle_samples_move_the_limit() {
    let logger = logger(
        ScriptedClient::with_replies(&[]),
        r#","s3_adaptive_concurrency":true,"s3_max_concurrent_uploads":2,"s3_max_adaptive_concurrency":4,"s3_max_queue_size":9,"s3_batch_file_upload":true"#,
        BTreeMap::new(),
        None,
        None,
    );
    assert_eq!(logger.settings.max_concurrent_uploads, 2);
    assert_eq!(logger.settings.max_queue_size, 9);
    assert!(logger.settings.batch_file_upload);
    assert_eq!(logger.uploads.snapshot().limit, 2);
    let throttle = logger
        .transport
        .throttle
        .as_ref()
        .expect("adaptive concurrency feeds the limiter");
    throttle.record(false);
    throttle.record(false);
    assert_eq!(logger.uploads.snapshot().limit, 3);
    drop(logger);
}

#[tokio::test]
async fn callback_logger_raises_the_adaptive_ceiling_to_the_window() {
    let logger = logger(
        ScriptedClient::with_replies(&[]),
        r#","s3_adaptive_concurrency":true,"s3_max_concurrent_uploads":3,"s3_max_adaptive_concurrency":2"#,
        BTreeMap::new(),
        None,
        None,
    );
    let throttle = logger
        .transport
        .throttle
        .as_ref()
        .expect("adaptive concurrency feeds the limiter");
    throttle.record(false);
    throttle.record(false);
    throttle.record(false);
    assert_eq!(logger.uploads.snapshot().limit, 3);
    drop(logger);
}

#[tokio::test]
async fn callback_logger_reuses_the_cold_storage_key_only_when_cold_storage_is_on() {
    let day_key = "2009-02-13/time-23-31-30-000000_s3-model-event.json";
    let text = PAYLOAD.replace(
        r#""metadata": {}"#,
        &format!(r#""metadata": {{"cold_storage_object_key": "{day_key}"}}"#),
    );
    let environment = BTreeMap::from([("S3_PARTITION_GRANULARITY".to_owned(), "hour".to_owned())]);
    let enabled = logger(
        ScriptedClient::with_replies(&[]),
        "",
        environment.clone(),
        Some("s3_v2"),
        None,
    );
    enabled
        .async_log_success_event(event(&text))
        .await
        .expect("the event is queued");
    assert_eq!(enabled.queue.snapshot().entries[0].s3_object_key, day_key);
    let disabled = logger(
        ScriptedClient::with_replies(&[]),
        "",
        environment,
        None,
        None,
    );
    disabled
        .async_log_success_event(event(&text))
        .await
        .expect("the event is queued");
    assert_eq!(
        disabled.queue.snapshot().entries[0].s3_object_key,
        "2009-02-13/23/time-23-31-30-000000_s3-model-event.json"
    );
    drop((enabled, disabled));
}

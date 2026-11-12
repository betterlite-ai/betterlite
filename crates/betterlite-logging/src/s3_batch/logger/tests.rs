#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use indexmap::IndexMap;
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
use crate::s3_batch::logger::{S3BatchDeps, S3BatchLogger};
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
    requests: Mutex<Vec<String>>,
}

impl ScriptedClient {
    fn with_replies(replies: &[u16]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.iter().copied().collect()),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl S3HttpClient for ScriptedClient {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.url);
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
    environment: BTreeMap<String, String>,
    tasks: Option<TaskTracker>,
    shutdown: CancellationToken,
) -> Arc<S3BatchLogger> {
    let lookup: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(environment);
    let parameters =
        resolve_s3_parameters::<()>(&Source::default(), &source(""), None, lookup.as_ref());
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

fn event() -> Arc<LoggerEvent> {
    let payload: StandardLoggingPayload =
        serde_json::from_str(PAYLOAD).expect("the payload is a standard logging payload");
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        stream: false,
        guardrail_snapshot: None,
    })
}

#[tokio::test]
async fn callback_logger_queues_model_events() {
    let client = ScriptedClient::with_replies(&[]);
    let logger = logger(
        Arc::clone(&client),
        BTreeMap::new(),
        None,
        CancellationToken::new(),
    );
    logger
        .async_log_success_event(event())
        .await
        .expect("the event is queued");
    logger
        .async_log_failure_event(event())
        .await
        .expect("the event is queued");
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
    assert_eq!(queued.entries[0].retrying_since, None);
    assert!(client.requests().is_empty());
    drop(logger);
}

#[tokio::test]
async fn callback_logger_uses_the_configured_partition_granularity() {
    let logger = logger(
        ScriptedClient::with_replies(&[]),
        BTreeMap::from([("S3_PARTITION_GRANULARITY".to_owned(), "hour".to_owned())]),
        None,
        CancellationToken::new(),
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
        BTreeMap::new(),
        Some(tracker.clone()),
        shutdown.clone(),
    );
    logger
        .async_log_success_event(event())
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
        BTreeMap::new(),
        Some(tracker.clone()),
        shutdown.clone(),
    );
    shutdown.cancel();
    tracker.close();
    timeout(Duration::from_secs(1), tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
    logger
        .async_log_success_event(event())
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
        BTreeMap::new(),
        None,
        CancellationToken::new(),
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
        .async_log_success_event(event())
        .await
        .expect("a model event shares the queue");
    assert_eq!(logger.queue.snapshot().entries.len(), 2);
    drop(logger);
}

#[tokio::test]
async fn callback_logger_reports_a_bad_payload_timestamp() {
    let logger = logger(
        ScriptedClient::with_replies(&[]),
        BTreeMap::new(),
        None,
        CancellationToken::new(),
    );
    let mut payload: StandardLoggingPayload =
        serde_json::from_str(PAYLOAD).expect("the payload loads");
    payload.start_time = rust_decimal::Decimal::MAX;
    let event = Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        stream: false,
        guardrail_snapshot: None,
    });
    let error = logger.async_log_success_event(event).await;
    error.expect_err("a payload past the timestamp range fails");
    assert!(logger.queue.snapshot().entries.is_empty());
    drop(logger);
}

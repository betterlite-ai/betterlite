use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_logging::s3_batch::Element;
use betterlite_logging::s3_batch::flush::MonotonicClock;
use betterlite_logging::s3_batch::logger::{S3BatchDeps, S3BatchLogger};
use betterlite_logging::s3_batch::ndjson_batch_file::{BatchIdSource, UuidBatchIds};
use betterlite_logging::s3_transport::batch_element::ContentType;
use betterlite_logging::s3_transport::error::S3TransportError;
use betterlite_logging::s3_transport::signing::{
    Clock, CredentialSource, Credentials, StaticCredentials,
};
use betterlite_logging::s3_transport::transport::{
    S3HttpClient, S3Request, S3Response, ThrottleSink,
};
use betterlite_logging::s3_v2::parameters::resolve_s3_parameters;
use betterlite_logging::s3_v2::source::Source;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    source: Source<()>,
    credentials: Keys,
    now: u64,
    #[serde(default)]
    cold_storage_logger: Option<String>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
pub(crate) struct Keys {
    access_key_id: String,
    secret_access_key: String,
    #[serde(default)]
    session_token: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct Step {
    #[serde(default)]
    enqueue: Vec<ElementInput>,
    #[serde(default)]
    replies: Vec<ReplyRule>,
}

#[derive(Deserialize)]
pub(crate) struct ElementInput {
    s3_object_key: String,
    s3_object_download_filename: String,
    payload: Json,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    content_type: ContentType,
    #[serde(default)]
    retrying_since_offset: Option<f64>,
}

#[derive(Deserialize)]
pub(crate) struct ReplyRule {
    #[serde(rename = "match")]
    key_match: String,
    responses: Vec<Reply>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Reply {
    status: u16,
    #[serde(default)]
    body: String,
}

#[derive(Debug)]
struct ScriptedS3 {
    rules: Mutex<Vec<ReplyRule>>,
    served: Mutex<BTreeMap<String, usize>>,
    in_flight: AtomicUsize,
    peak: AtomicUsize,
    requests: Mutex<Vec<(String, String)>>,
}

impl S3HttpClient for ScriptedS3 {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        let running = self
            .in_flight
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        self.peak.fetch_max(running, Ordering::SeqCst);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                request.url.clone(),
                String::from_utf8(request.body).unwrap_or_default(),
            ));
        let reply = self.reply_for(&request.url);
        Box::pin(async move {
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(S3Response {
                status: reply.status,
                body: reply.body.into_bytes(),
            })
        })
    }
}

impl ScriptedS3 {
    fn reply_for(&self, url: &str) -> Reply {
        let rules = self.rules.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(rule) = rules.iter().find(|rule| url.contains(&rule.key_match)) else {
            return Reply {
                status: 200,
                body: String::new(),
            };
        };
        let mut served = self.served.lock().unwrap_or_else(PoisonError::into_inner);
        let index = served.entry(rule.key_match.clone()).or_insert(0);
        let reply = rule.responses.get(*index).cloned().unwrap_or(Reply {
            status: 200,
            body: String::new(),
        });
        *index = index.saturating_add(1);
        reply
    }

    fn reset(&self, replies: &[ReplyRule]) {
        *self.rules.lock().unwrap_or_else(PoisonError::into_inner) = replies
            .iter()
            .map(|rule| ReplyRule {
                key_match: rule.key_match.clone(),
                responses: rule.responses.clone(),
            })
            .collect();
        self.served
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.in_flight.store(0, Ordering::SeqCst);
        self.peak.store(0, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[derive(Debug, Default)]
struct ManualMonotonic;

impl MonotonicClock for ManualMonotonic {
    fn seconds(&self) -> f64 {
        0.0_f64
    }
}

#[derive(Debug, Default)]
struct RecordingSamples(Mutex<Vec<bool>>);

impl ThrottleSink for RecordingSamples {
    fn record(&self, throttled: bool) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(throttled);
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(|error| failure("RuntimeError", &error.to_string()))?;
        runtime.block_on(execute(input))
    }
}

fn failure(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

async fn execute(input: &Input) -> HelperResult {
    let environment: Arc<dyn betterlite_config::loader::EnvironmentLookup + Send + Sync> =
        Arc::new(BTreeMap::<String, String>::new());
    let parameters = resolve_s3_parameters::<()>(
        &Source::default(),
        &input.source,
        None,
        environment.as_ref(),
    );
    let client = Arc::new(ScriptedS3 {
        rules: Mutex::new(Vec::new()),
        served: Mutex::new(BTreeMap::new()),
        in_flight: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        requests: Mutex::new(Vec::new()),
    });
    let samples = Arc::new(RecordingSamples::default());
    let adaptive = parameters
        .features
        .contains(&betterlite_logging::s3_v2::Feature::AdaptiveConcurrency);
    let logger = S3BatchLogger::new(
        parameters,
        S3BatchDeps {
            client: Arc::clone(&client) as Arc<dyn S3HttpClient>,
            credentials: Arc::new(StaticCredentials {
                credentials: Credentials {
                    access_key_id: SecretString::from(input.credentials.access_key_id.clone()),
                    secret_access_key: SecretString::from(
                        input.credentials.secret_access_key.clone(),
                    ),
                    session_token: input
                        .credentials
                        .session_token
                        .as_ref()
                        .map(|token| SecretString::from(token.clone())),
                },
            }),
            clock: Arc::new(FixedClock(
                SystemTime::UNIX_EPOCH
                    .checked_add(Duration::from_secs(input.now))
                    .unwrap_or_default(),
            )),
            monotonic: Arc::new(ManualMonotonic),
            batch_ids: Arc::new(UuidBatchIds),
            environment,
            throttle_observer: adaptive.then(|| Arc::clone(&samples) as Arc<dyn ThrottleSink>),
            cold_storage_logger: input.cold_storage_logger.clone(),
            tasks: None,
            shutdown: CancellationToken::new(),
        },
    )
    .map_err(|error| failure("InitError", &error.to_string()))?;
    let mut steps = Vec::new();
    for step in &input.steps {
        client.reset(&step.replies);
        for element in &step.enqueue {
            logger.queue.enqueue(element_from(element));
        }
        logger.flush_queue().await.ok();
        let snapshot = logger.queue.snapshot();
        let mut requests = client
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        requests.sort();
        client
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        steps.push(json!({
            "requests": requests.iter().map(|(url, body)| json!({"url": url, "body": body})).collect::<Vec<_>>(),
            "queue": snapshot.entries.iter().map(queue_entry).collect::<Vec<_>>(),
            "requeued_count": snapshot.requeued_count,
            "samples": adaptive.then(|| samples.0.lock().unwrap_or_else(PoisonError::into_inner).clone()),
            "limit": logger.uploads.snapshot().limit,
            "peak": client.peak.load(Ordering::SeqCst),
        }));
    }
    Ok(json!({"steps": steps}))
}

fn element_from(input: &ElementInput) -> Element {
    Element {
        payload: input.payload.clone(),
        s3_object_key: input.s3_object_key.clone(),
        s3_object_download_filename: input.s3_object_download_filename.clone(),
        body: input.body.clone(),
        content_type: input.content_type,
        retrying_since: input.retrying_since_offset,
    }
}

fn queue_entry(element: &Element) -> Value {
    json!({
        "key": element.s3_object_key,
        "retrying_since": element.retrying_since.is_some(),
        "body": element.body,
    })
}

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use aws_credential_types::Credentials;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::sqs_logging::clock::Clock;
use crate::sqs_logging::credentials::{CredentialError, CredentialParams, CredentialSource};
use crate::sqs_logging::health::{DummyHealthPayload, HEALTH_CHECK_PAYLOAD, HealthPayloadFactory};
use crate::sqs_logging::settings::{Overrides, Settings, SettingsError};
use crate::sqs_logging::{SqsDeps, SqsLogger};

pub(crate) const QUEUE_URL: &str = "https://sqs.us-east-1.amazonaws.com/123456789012/test-queue";
pub(crate) const ACCESS_KEY_ID: &str = "AKIAIOSFODNN7EXAMPLE";
pub(crate) const SECRET_ACCESS_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
pub(crate) const FIXED_TIME_SECONDS: u64 = 1_767_323_045;

pub(crate) fn fixed_time() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(FIXED_TIME_SECONDS))
        .expect("the fixed time is representable")
}

pub(crate) struct FixedClock(pub(crate) SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

fn resolved(token: Option<&str>) -> Credentials {
    Credentials::new(
        ACCESS_KEY_ID,
        SECRET_ACCESS_KEY,
        token.map(str::to_owned),
        None,
        "test",
    )
}

pub(crate) struct StaticCredentials;

#[async_trait]
impl CredentialSource for StaticCredentials {
    fn validate(&self, _params: &CredentialParams) -> Result<(), CredentialError> {
        Ok(())
    }

    async fn resolve(&self, _params: &CredentialParams) -> Result<Credentials, CredentialError> {
        Ok(resolved(None))
    }
}

pub(crate) struct SessionCredentials;

#[async_trait]
impl CredentialSource for SessionCredentials {
    fn validate(&self, _params: &CredentialParams) -> Result<(), CredentialError> {
        Ok(())
    }

    async fn resolve(&self, _params: &CredentialParams) -> Result<Credentials, CredentialError> {
        Ok(resolved(Some("SESSIONTOKEN")))
    }
}

#[derive(Default)]
pub(crate) struct RecordedCredentials {
    pub(crate) calls: Mutex<Vec<CredentialParams>>,
}

#[async_trait]
impl CredentialSource for RecordedCredentials {
    fn validate(&self, _params: &CredentialParams) -> Result<(), CredentialError> {
        Ok(())
    }

    async fn resolve(&self, params: &CredentialParams) -> Result<Credentials, CredentialError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(params.clone());
        Ok(resolved(None))
    }
}

#[derive(Clone)]
pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) headers: http::HeaderMap,
    pub(crate) body: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct Records {
    pub(crate) entries: Mutex<Vec<Recorded>>,
    notify: Notify,
}

impl Records {
    pub(crate) async fn wait_for(&self, count: usize) {
        loop {
            let notified = self.notify.notified();
            let recorded = self
                .entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .len();
            if recorded >= count {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(crate) fn snapshot(&self) -> Vec<Recorded> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn body(&self, index: usize) -> String {
        let entries = self.snapshot();
        String::from_utf8(entries[index].body.clone()).expect("the body is UTF-8")
    }
}

pub(crate) enum Outcome {
    Delivered { status: u16, body: Vec<u8> },
    Failed,
}

pub(crate) struct RecordingDelivery {
    pub(crate) records: Arc<Records>,
    pub(crate) outcome: Outcome,
}

impl RecordingDelivery {
    pub(crate) fn delivering() -> Self {
        Self::answering_bytes(200, Vec::new())
    }

    pub(crate) fn answering(status: u16, body: &str) -> Self {
        Self::answering_bytes(status, body.as_bytes().to_vec())
    }

    pub(crate) fn answering_bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            records: Arc::new(Records::default()),
            outcome: Outcome::Delivered { status, body },
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            records: Arc::new(Records::default()),
            outcome: Outcome::Failed,
        }
    }
}

impl Delivery for RecordingDelivery {
    type Error = TransportError;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, TransportError>> + Send {
        self.records
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.to_string(),
                headers: request.headers,
                body: request.body,
            });
        self.records.notify.notify_waiters();
        ready(match &self.outcome {
            Outcome::Delivered { status, body } => Ok(PostResponse {
                status: *status,
                body: body.clone(),
            }),
            Outcome::Failed => Err(TransportError::Closed),
        })
    }
}

pub(crate) struct FailingHealth;

impl HealthPayloadFactory for FailingHealth {
    fn build(&self) -> Result<StandardLoggingPayload, serde_json::Error> {
        Err(serde_json::from_str::<StandardLoggingPayload>("{").expect_err("invalid JSON"))
    }
}

pub(crate) struct Harness {
    pub(crate) settings: Settings,
    pub(crate) credentials: Arc<dyn CredentialSource>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) health_payload: Arc<dyn HealthPayloadFactory>,
    pub(crate) delivery: RecordingDelivery,
}

impl Harness {
    pub(crate) fn standard() -> Self {
        Self {
            settings: settings(),
            credentials: Arc::new(StaticCredentials),
            clock: Arc::new(FixedClock(fixed_time())),
            health_payload: Arc::new(DummyHealthPayload),
            delivery: RecordingDelivery::delivering(),
        }
    }

    pub(crate) fn build(self) -> (Arc<SqsLogger<RecordingDelivery>>, Arc<Records>) {
        let records = Arc::clone(&self.delivery.records);
        let logger = SqsLogger::new(
            self.settings,
            SqsDeps {
                delivery: self.delivery,
                credentials: self.credentials,
                clock: self.clock,
                health_payload: self.health_payload,
                tasks: TaskTracker::new(),
            },
        )
        .expect("the SQS logger builds");
        (Arc::new(logger), records)
    }
}

pub(crate) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn settings() -> Settings {
    settings_with(&[
        ("sqs_queue_url", QUEUE_URL),
        ("sqs_region_name", "us-east-1"),
        ("sqs_aws_access_key_id", ACCESS_KEY_ID),
        ("sqs_aws_secret_access_key", SECRET_ACCESS_KEY),
    ])
}

pub(crate) fn settings_with(pairs: &[(&str, &str)]) -> Settings {
    settings_from_environment(pairs, &[]).expect("the SQS settings resolve")
}

pub(crate) fn settings_from_environment(
    pairs: &[(&str, &str)],
    variables: &[(&str, &str)],
) -> Result<Settings, SettingsError> {
    let params: IndexMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let environment = environment(variables);
    Settings::resolve(&params, Overrides::default(), &environment)
}

pub(crate) fn payload() -> StandardLoggingPayload {
    serde_json::from_str(HEALTH_CHECK_PAYLOAD)
        .expect("the health-check payload is a standard logging payload")
}

pub(crate) fn payload_with_messages(messages: &str) -> StandardLoggingPayload {
    let mut standard = payload();
    standard.messages =
        Some(serde_json::from_str(messages).expect("the messages fixture is valid JSON"));
    standard
}

pub(crate) fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

#[derive(Clone, Default)]
pub(crate) struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    pub(crate) fn contents(&self) -> String {
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

pub(crate) async fn logged<Action>(action: Action) -> (Action::Output, String)
where
    Action: Future,
{
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    let output = action.await;
    drop(guard);
    (output, buffer.contents())
}

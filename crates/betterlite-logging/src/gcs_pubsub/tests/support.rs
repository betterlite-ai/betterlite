use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::batch::logger::{BatchSettings, DEFAULT_MAX_QUEUE_SIZE};
use crate::callback_logger::event::LoggerEvent;
use crate::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth,
};
use crate::gcs_pubsub::{GcsPubSubDeps, GcsPubSubLogger, Overrides, PubSubError};
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::registry::catalog::gcs_pubsub::ProjectionError;
use crate::registry::catalog::gcs_pubsub::payload::spend_log_payload;

pub(crate) const TOKEN: &str = "pubsub-test-token";
pub(crate) const PROJECT: &str = "pubsub-project";
pub(crate) const TOPIC: &str = "spend-logs";
pub(crate) const CREDENTIALS: &str = "/keys/service-account.json";

pub(crate) const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": "call-1",
    "call_type": "completion",
    "stream": false,
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "custom_llm_provider": "openai",
    "api_base": "https://api.openai.com",
    "metadata": {
        "user_api_key_user_id": "user-1",
        "user_api_key_end_user_id": "end-user-1"
    },
    "cache_hit": false,
    "saved_cache_cost": 0.0,
    "request_tags": ["team-a"],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": {
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1704067200,
        "model": "gpt-4o",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
    },
    "error_str": null
}"#;

#[derive(Debug, thiserror::Error)]
#[error("the test authentication failed")]
pub(crate) struct AuthFailure;

#[derive(Clone, Debug, Default)]
pub(crate) struct FakeAuth {
    access: Arc<Mutex<Vec<EnsureAccessToken>>>,
    tokens: Arc<Mutex<Vec<GetTokenAndUrl>>>,
}

impl FakeAuth {
    pub(crate) fn access_requests(&self) -> Vec<EnsureAccessToken> {
        self.access
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn token_requests(&self) -> Vec<GetTokenAndUrl> {
        self.tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl VertexAuth for FakeAuth {
    type Error = AuthFailure;

    async fn ensure_access_token_async(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, AuthFailure> {
        self.access
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        Ok(access_token(request))
    }

    fn ensure_access_token(&self, request: EnsureAccessToken) -> Result<AccessToken, AuthFailure> {
        Ok(access_token(request))
    }

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, AuthFailure> {
        self.tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        Ok(TokenAndUrl {
            token: request
                .auth_header
                .unwrap_or_else(|| SecretString::from(TOKEN.to_owned())),
            url: String::new(),
        })
    }
}

fn access_token(request: EnsureAccessToken) -> AccessToken {
    AccessToken {
        token: SecretString::from(TOKEN.to_owned()),
        project_id: request.project_id.unwrap_or_default(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Json,
}

#[derive(Debug, thiserror::Error)]
#[error("the test transport failed")]
pub(crate) struct TransportFailure;

#[derive(Clone, Debug)]
pub(crate) struct Captured {
    requests: Arc<Mutex<Vec<Recorded>>>,
    answers: Arc<Mutex<VecDeque<u16>>>,
    failing: Arc<AtomicBool>,
    fallback: u16,
}

impl Default for Captured {
    fn default() -> Self {
        Self {
            requests: Arc::default(),
            answers: Arc::default(),
            failing: Arc::default(),
            fallback: 200,
        }
    }
}

impl Captured {
    pub(crate) fn answer(&self, status: u16) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(status);
    }

    pub(crate) fn fail(&self) {
        self.failing.store(true, Ordering::SeqCst);
    }

    pub(crate) fn recover(&self) {
        self.failing.store(false, Ordering::SeqCst);
    }

    pub(crate) fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Delivery for Captured {
    type Error = TransportFailure;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        let headers = request
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_owned(), text.to_owned()))
            })
            .collect();
        let body = serde_json::from_slice::<Json>(&request.body).unwrap_or(Json::Null);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.to_string(),
                headers,
                body,
            });
        if self.failing.load(Ordering::SeqCst) {
            return ready(Err(TransportFailure));
        }
        let status = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(self.fallback);
        ready(Ok(PostResponse {
            status,
            body: Vec::new(),
        }))
    }
}

pub(crate) struct Fixture {
    pub(crate) logger: TestLogger,
    pub(crate) auth: FakeAuth,
    pub(crate) delivery: Captured,
}

pub(crate) type TestLogger = GcsPubSubLogger<FakeAuth, Captured>;

pub(crate) fn assemble(
    environment: &[(&str, &str)],
    overrides: Overrides,
    batch_size: usize,
    project: fn(&LoggerEvent) -> Result<Json, ProjectionError>,
) -> Result<(TestLogger, FakeAuth, Captured), PubSubError<FakeAuth, Captured>> {
    let variables: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let auth = FakeAuth::default();
    let delivery = Captured::default();
    let logger = GcsPubSubLogger::new(
        &variables,
        overrides,
        GcsPubSubDeps {
            auth: auth.clone(),
            delivery: delivery.clone(),
            project,
            batch: BatchSettings {
                batch_size,
                flush_interval: Duration::from_secs(5),
                max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
            },
        },
    )?;
    Ok((logger, auth, delivery))
}

pub(crate) fn try_open(
    environment: &[(&str, &str)],
    overrides: Overrides,
) -> Result<TestLogger, PubSubError<FakeAuth, Captured>> {
    Ok(assemble(environment, overrides, 512, spend_log_payload)?.0)
}

pub(crate) fn open(
    environment: &[(&str, &str)],
    overrides: Overrides,
    batch_size: usize,
) -> Fixture {
    open_with(environment, overrides, batch_size, spend_log_payload)
}

pub(crate) fn open_with(
    environment: &[(&str, &str)],
    overrides: Overrides,
    batch_size: usize,
    project: fn(&LoggerEvent) -> Result<Json, ProjectionError>,
) -> Fixture {
    let (logger, auth, delivery) =
        assemble(environment, overrides, batch_size, project).expect("the logger builds");
    Fixture {
        logger,
        auth,
        delivery,
    }
}

pub(crate) fn configured() -> [(&'static str, &'static str); 2] {
    [
        ("GCS_PUBSUB_PROJECT_ID", PROJECT),
        ("GCS_PUBSUB_TOPIC_ID", TOPIC),
    ]
}

pub(crate) fn event_from(payload: &str) -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(serde_json::from_str(payload).expect("the fixture payload decodes")),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}

pub(crate) fn event() -> LoggerEvent {
    event_from(PAYLOAD)
}

pub(crate) fn event_with_metadata(pairs: &[(&str, &str)]) -> LoggerEvent {
    let mut event = event();
    let merged: IndexMap<String, Json> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Json::Text((*value).to_owned())))
        .collect();
    event.litellm_metadata = Arc::new(merged);
    event
}

pub(crate) fn messages(recorded: &Recorded) -> Vec<String> {
    let Json::Object(body) = &recorded.body else {
        panic!("the publish body is an object");
    };
    let Some(Json::List(entries)) = body.get("messages") else {
        panic!("the publish body carries messages");
    };
    entries.iter().map(message_text).collect()
}

fn message_text(entry: &Json) -> String {
    let Json::Object(fields) = entry else {
        panic!("a message entry is an object");
    };
    let Some(Json::Text(data)) = fields.get("data") else {
        panic!("a message entry carries data");
    };
    let bytes = STANDARD.decode(data).expect("the data is base64");
    String::from_utf8(bytes).expect("the data is UTF-8")
}

pub(crate) fn published(recorded: &Recorded) -> Json {
    let text = messages(recorded).first().cloned().unwrap_or_default();
    serde_json::from_str(&text).expect("the published message is JSON")
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the log output is UTF-8")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (DefaultGuard, LogBuffer) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (set_default(subscriber), buffer)
}

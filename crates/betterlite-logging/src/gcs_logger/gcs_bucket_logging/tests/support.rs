use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;
use url::Url;

use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;
use crate::gcs_client::gcs_storage_auth::GcsBucketBase;
use crate::gcs_client::gcs_storage_auth::initialization_behaviors::initialize;
use crate::gcs_logger::gcs_bucket_logging::event_enqueue_behaviors::enqueue;
use crate::gcs_logger::gcs_bucket_logging::initialization_behaviors::start;
use crate::gcs_logger::gcs_bucket_logging::{GcsBucketLogger, GcsDeps, QueuedItem};
use crate::gcs_logger::runtime::IdSource;
use crate::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth,
};
use crate::gcs_mock::client::{GcsHttpClient, GcsHttpResponse};
use crate::gcs_mock::request::GcsRequest;
use crate::prometheus_query::clock::Clock;

pub(crate) const TOKEN: &str = "test-token";

#[derive(Debug, thiserror::Error)]
#[error("the test authentication failed")]
pub(crate) struct AuthFailure;

#[derive(Debug, thiserror::Error)]
#[error("the test transport failed")]
pub(crate) struct TransportFailure;

#[derive(Clone, Debug, Default)]
pub(crate) struct FakeAuth;

#[async_trait]
impl VertexAuth for FakeAuth {
    type Error = AuthFailure;

    async fn ensure_access_token_async(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, AuthFailure> {
        Ok(access_token(request))
    }

    fn ensure_access_token(&self, request: EnsureAccessToken) -> Result<AccessToken, AuthFailure> {
        Ok(access_token(request))
    }

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, AuthFailure> {
        Ok(TokenAndUrl {
            token: request
                .auth_header
                .unwrap_or_else(|| SecretString::from(TOKEN.to_owned())),
            url: "https://storage.googleapis.com".to_owned(),
        })
    }
}

fn access_token(request: EnsureAccessToken) -> AccessToken {
    AccessToken {
        token: SecretString::from(TOKEN.to_owned()),
        project_id: request.project_id.unwrap_or_default(),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

impl Answer {
    pub(crate) fn ok(body: &str) -> Self {
        Self {
            status: 200,
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn with_status(status: u16) -> Self {
        Self {
            status,
            body: b"{}".to_vec(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RecordingClient {
    pub(crate) sent: Arc<Mutex<Vec<GcsRequest>>>,
    answers: Arc<Mutex<VecDeque<Answer>>>,
    failing: Arc<AtomicBool>,
    fallback: Answer,
}

impl RecordingClient {
    pub(crate) fn new(fallback: Answer) -> Self {
        Self {
            sent: Arc::default(),
            answers: Arc::default(),
            failing: Arc::default(),
            fallback,
        }
    }

    pub(crate) fn fail_sends(&self) {
        self.failing.store(true, Ordering::SeqCst);
    }

    pub(crate) fn script(&self, answers: Vec<Answer>) {
        let mut queue = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        queue.clear();
        queue.extend(answers);
    }

    pub(crate) fn requests(&self) -> Vec<GcsRequest> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl GcsHttpClient for RecordingClient {
    type Error = TransportFailure;

    async fn send(&self, request: GcsRequest) -> Result<GcsHttpResponse, TransportFailure> {
        let url = request.url().to_owned();
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if self.failing.load(Ordering::SeqCst) {
            return Err(TransportFailure);
        }
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| self.fallback.clone());
        Ok(GcsHttpResponse {
            status: answer.status,
            headers: HeaderMap::new(),
            body: answer.body,
            url,
            elapsed: Duration::ZERO,
        })
    }
}

#[derive(Clone)]
pub(crate) struct SetClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl SetClock {
    pub(crate) fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    pub(crate) fn set(&self, now: DateTime<Utc>) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) = now;
    }
}

impl Clock for SetClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Clone, Default)]
pub(crate) struct SequenceIds {
    next: Arc<AtomicUsize>,
}

impl IdSource for SequenceIds {
    fn uuid_hex(&self) -> String {
        format!("{:032x}", self.next.fetch_add(1, Ordering::SeqCst))
    }
}

pub(crate) struct Harness {
    pub(crate) logger: Arc<GcsBucketLogger<FakeAuth, RecordingClient>>,
    pub(crate) client: RecordingClient,
    pub(crate) clock: SetClock,
}

impl Harness {
    pub(crate) async fn enqueue_all(&self, ids: &[&str]) {
        for id in ids {
            enqueue(&self.logger, &event(id, None, Json::Null, IndexMap::new())).await;
        }
    }

    pub(crate) fn requests(&self) -> Vec<GcsRequest> {
        self.client.requests()
    }

    pub(crate) fn queue_len(&self) -> usize {
        self.logger
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(crate) fn queued(&self) -> Vec<QueuedItem> {
        self.logger
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    pub(crate) fn last_flush_time(&self) -> DateTime<Utc> {
        *self
            .logger
            .last_flush_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) fn open(pairs: &[(&str, &str)], bucket: Option<&str>, fallback: Answer) -> Harness {
    build(pairs, bucket, fallback, None)
}

pub(crate) fn open_scheduled(
    pairs: &[(&str, &str)],
    bucket: Option<&str>,
    fallback: Answer,
    tracker: &TaskTracker,
    shutdown: &CancellationToken,
) -> Harness {
    build(pairs, bucket, fallback, Some((tracker, shutdown)))
}

fn build(
    pairs: &[(&str, &str)],
    bucket: Option<&str>,
    fallback: Answer,
    schedule: Option<(&TaskTracker, &CancellationToken)>,
) -> Harness {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let client = RecordingClient::new(fallback);
    let base: GcsBucketBase<FakeAuth, RecordingClient> = initialize(
        Arc::new(environment),
        bucket.map(str::to_owned),
        FakeAuth,
        client.clone(),
    )
    .expect("the GCS base initializes");
    let clock = SetClock::new(now());
    let ids = SequenceIds::default();
    let (tasks, shutdown) = match schedule {
        Some((tracker, token)) => (Some(tracker.clone()), token.clone()),
        None => (None, CancellationToken::new()),
    };
    let logger = start(
        base,
        GcsDeps {
            clock: Arc::new(clock.clone()),
            ids: Arc::new(ids),
            tasks,
            shutdown,
        },
    )
    .expect("the GCS logger starts");
    Harness {
        logger,
        client,
        clock,
    }
}

pub(crate) fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2024-03-09T23:59:59Z")
        .expect("the fixed time is valid")
        .with_timezone(&Utc)
}

const PAYLOAD: &str = r#"{
  "id": "chatcmpl-1",
  "trace_id": "trace-1",
  "session_id": "session-1",
  "litellm_call_id": null,
  "call_type": "completion",
  "stream": null,
  "response_cost": 0.5,
  "cost_breakdown": null,
  "autorouter_savings": null,
  "autorouter_savings_estimate": null,
  "autorouter_baseline_observation": null,
  "response_cost_failure_debug_info": null,
  "status": "success",
  "status_fields": {
    "llm_api_status": "success",
    "guardrail_status": "not_run"
  },
  "custom_llm_provider": null,
  "total_tokens": 0,
  "prompt_tokens": 0,
  "completion_tokens": 0,
  "startTime": 0.5,
  "endTime": 0.5,
  "completionStartTime": 0.5,
  "response_time": 0.5,
  "model_map_information": {
    "model_map_key": "gpt-4o",
    "model_map_value": null
  },
  "model": "gpt-4o",
  "model_id": null,
  "model_group": null,
  "api_base": "https://api.openai.com",
  "metadata": {
    "user_api_key_hash": null,
    "user_api_key_alias": null,
    "user_api_key_spend": null,
    "user_api_key_max_budget": null,
    "user_api_key_budget_reset_at": null,
    "user_api_key_user_spend": null,
    "user_api_key_user_max_budget": null,
    "user_api_key_team_spend": null,
    "user_api_key_team_max_budget": null,
    "user_api_key_org_id": null,
    "user_api_key_org_alias": null,
    "user_api_key_team_id": null,
    "user_api_key_project_id": null,
    "user_api_key_project_alias": null,
    "user_api_key_user_id": null,
    "user_api_key_user_email": null,
    "user_api_key_team_alias": null,
    "user_api_key_end_user_id": null,
    "user_api_key_request_route": null,
    "user_api_key_auth_metadata": null,
    "spend_logs_metadata": null,
    "requester_ip_address": null,
    "user_agent": null,
    "requester_metadata": null,
    "requester_custom_headers": null,
    "prompt_management_metadata": null,
    "mcp_tool_call_metadata": null,
    "vector_store_request_metadata": null,
    "routing_decision": null,
    "applied_guardrails": null,
    "usage_object": null,
    "cold_storage_object_key": null,
    "team_alias": null,
    "team_id": null,
    "used_client_oauth_token": null
  },
  "cache_hit": null,
  "cache_key": null,
  "saved_cache_cost": 0.5,
  "request_tags": [],
  "end_user": null,
  "requester_ip_address": null,
  "user_agent": null,
  "messages": null,
  "response": null,
  "error_str": null,
  "error_information": null,
  "model_parameters": {},
  "hidden_params": {
    "model_id": null,
    "cache_key": null,
    "api_base": null,
    "response_cost": null,
    "litellm_overhead_time_ms": null,
    "additional_headers": null,
    "batch_models": null,
    "batch_successful_requests": null,
    "batch_failed_requests": null,
    "litellm_model_name": null,
    "usage_object": null
  },
  "guardrail_information": null,
  "standard_built_in_tools_params": null
}"#;

pub(crate) fn payload(id: &str, error: Option<&str>) -> StandardLoggingPayload {
    let mut built: StandardLoggingPayload =
        serde_json::from_str(PAYLOAD).expect("the fixture payload parses");
    if error.is_some() {
        built.status = LoggingStatus::Failure;
        built.status_fields.llm_api_status = LoggingStatus::Failure;
    }
    built.id = id.to_owned();
    built.response = Some(Json::Object(IndexMap::from([(
        "id".to_owned(),
        Json::Text(id.to_owned()),
    )])));
    built.error_str = error.map(str::to_owned);
    built
}

pub(crate) fn event(
    id: &str,
    error: Option<&str>,
    dynamic: Json,
    metadata: IndexMap<String, Json>,
) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload(id, error)),
        dynamic_params: Arc::new(dynamic),
        litellm_metadata: Arc::new(metadata),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(crate) fn event_with_response(response: Json) -> Arc<LoggerEvent> {
    let mut standard = payload("chatcmpl-1", None);
    standard.response = Some(response);
    Arc::new(LoggerEvent {
        payload: Arc::new(standard),
        dynamic_params: Arc::new(Json::Null),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(crate) fn item(id: &str, error: Option<&str>, dynamic: Json) -> QueuedItem {
    QueuedItem {
        payload: Arc::new(payload(id, error)),
        dynamic_params: Arc::new(dynamic),
        metadata: Arc::new(IndexMap::new()),
        response_id: id.to_owned(),
    }
}

pub(crate) fn dynamic(pairs: &[(&str, &str)]) -> Json {
    Json::Object(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), Json::Text((*value).to_owned())))
            .collect(),
    )
}

pub(crate) fn metadata(key: &str, value: Json) -> IndexMap<String, Json> {
    IndexMap::from([(key.to_owned(), value)])
}

pub(crate) fn upload_name(request: &GcsRequest) -> Option<String> {
    let url = Url::parse(request.url()).ok()?;
    url.query_pairs()
        .find(|(key, _)| key == "name")
        .map(|(_, value)| value.into_owned())
}

pub(crate) fn body_text(request: &GcsRequest) -> Option<String> {
    let GcsRequest::Post(write) = request else {
        return None;
    };
    let Json::Text(text) = write.data.as_ref()? else {
        return None;
    };
    Some(text.clone())
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("captured logs are UTF-8")
    }
}

impl io::Write for CapturedLogs {
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

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (CapturedLogs, DefaultGuard) {
    let captured = CapturedLogs::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

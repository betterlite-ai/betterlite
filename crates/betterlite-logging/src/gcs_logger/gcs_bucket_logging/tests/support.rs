use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use url::Url;

use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::payload::records::{StandardLoggingHiddenParams, StandardLoggingMetadata};
use crate::fields::status::{GuardrailStatus, LoggingStatus, StatusFields};
use crate::gcs_client::gcs_storage_auth::GcsBucketBase;
use crate::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use crate::gcs_client::gcs_storage_auth::initialization_behaviors::initialize;
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

#[derive(Clone, Default)]
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

#[derive(Clone)]
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

pub(crate) fn payload(id: &str, error: Option<&str>) -> StandardLoggingPayload {
    let status = if error.is_some() {
        LoggingStatus::Failure
    } else {
        LoggingStatus::Success
    };
    let response = Json::Object(IndexMap::from([(
        "id".to_owned(),
        Json::Text(id.to_owned()),
    )]));
    StandardLoggingPayload {
        classifier_input: None,
        originating_request_masked: None,
        id: id.to_owned(),
        trace_id: "trace-1".to_owned(),
        session_id: "session-1".to_owned(),
        litellm_call_id: Some("call-1".to_owned()),
        call_type: "completion".to_owned(),
        stream: Some(false),
        response_cost: Decimal::new(5, 1),
        cost_breakdown: None,
        autorouter_savings: None,
        autorouter_savings_estimate: None,
        baseline_observation: None,
        cost_failure_info: None,
        zero_cost_diagnostic: None,
        status,
        status_fields: StatusFields {
            llm_api_status: status,
            guardrail_status: GuardrailStatus::NotRun,
        },
        custom_llm_provider: Some("openai".to_owned()),
        total_tokens: 5,
        prompt_tokens: 2,
        completion_tokens: 3,
        start_time: Decimal::new(17_040_672_000, 1),
        end_time: Decimal::new(17_040_672_015, 1),
        completion_start_time: Decimal::new(17_040_672_005, 1),
        response_time: Decimal::new(15, 1),
        model_map_information: IndexMap::new(),
        model: "gpt-4o".to_owned(),
        model_id: Some("model-1".to_owned()),
        model_group: None,
        api_base: "https://api.openai.com".to_owned(),
        metadata: StandardLoggingMetadata::default(),
        cache_hit: None,
        cache_key: None,
        saved_cache_cost: Decimal::ZERO,
        request_tags: Vec::new(),
        request_model_access_groups: None,
        end_user: None,
        requester_ip_address: None,
        user_agent: None,
        messages: None,
        response: Some(response),
        error_str: error.map(str::to_owned),
        error_information: None,
        model_parameters: IndexMap::new(),
        hidden_params: StandardLoggingHiddenParams::default(),
        guardrail_information: None,
        standard_built_in_tools_params: None,
    }
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

pub(crate) fn body_of(request: &GcsRequest) -> Option<Json> {
    let GcsRequest::Post(write) = request else {
        return None;
    };
    let Json::Text(text) = write.data.as_ref()? else {
        return None;
    };
    serde_json::from_str::<Json>(text).ok()
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

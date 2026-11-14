use std::collections::{BTreeMap, VecDeque};
use std::io::Read as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use flate2::read::GzDecoder;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use crate::callback_logger::event::LoggerEvent;
use crate::datadog::metrics::datadog_metrics::{DatadogMetricsLogger, MetricsDeps};
use crate::datadog::metrics::measurements::{MetricSource, RequestTiming, metrics_from_log};
use crate::datadog::metrics::series::DatadogMetricSeries;
pub(super) use crate::datadog::metrics::settings::MetricsInit;
use crate::datadog_logging::clock::Clock;
use crate::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::payload::records::ErrorInformation;
use crate::fields::status::LoggingStatus;

pub(super) type Environment = BTreeMap<String, String>;

pub(super) const FLUSH_INTERVAL: u64 = 5;
pub(super) const NOW: i64 = 1_700_000_000;
// the same instant as NOW, as a Duration for the fixed clock
pub(super) const NOW_DURATION: Duration = Duration::from_secs(1_700_000_000);

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067202.5,
    "completionStartTime": 1704067200.5,
    "response_time": 2.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "model_group": "production",
    "custom_llm_provider": "openai",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

pub(super) fn environment(pairs: &[(&str, &str)]) -> Environment {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).expect("the fixture is a standard logging payload")
}

pub(super) fn failure_payload(error_code: Option<&str>) -> StandardLoggingPayload {
    let mut payload = payload();
    payload.status = LoggingStatus::Failure;
    payload.status_fields.llm_api_status = LoggingStatus::Failure;
    payload.error_information = Some(ErrorInformation {
        error_code: Some(error_code.map(str::to_owned)),
        error_class: None,
        llm_provider: None,
        traceback: None,
        error_message: None,
        error_provider_request_id: None,
        error_rate_limit_category: None,
        error_rate_limit_type: None,
        error_budget_entity_type: None,
        error_budget_entity_id: None,
        normalized_error: None,
        error_budget_limit: None,
        error_budget_spend: None,
    });
    payload
}

pub(super) fn failure_without_information() -> StandardLoggingPayload {
    let mut payload = payload();
    payload.status = LoggingStatus::Failure;
    payload.status_fields.llm_api_status = LoggingStatus::Failure;
    payload
}

pub(super) fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(super) fn source<'source>(
    logger: &'source DatadogMetricsLogger,
    payload: &'source StandardLoggingPayload,
    status_code: &str,
) -> MetricSource<'source> {
    MetricSource {
        environment: &logger.settings.environment,
        payload,
        timing: RequestTiming {
            start_time: Some(payload.start_time),
            end_time: Some(payload.end_time),
            api_call_start_time: None,
        },
        status_code: status_code.to_owned(),
    }
}

pub(super) fn metrics_for(
    logger: &DatadogMetricsLogger,
    payload: &StandardLoggingPayload,
    status_code: &str,
    timing: RequestTiming,
) -> Vec<DatadogMetricSeries> {
    let source = MetricSource {
        environment: &logger.settings.environment,
        payload,
        timing,
        status_code: status_code.to_owned(),
    };
    metrics_from_log(&source, FLUSH_INTERVAL, Decimal::from(NOW)).expect("the metrics build")
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Outcome {
    Reply(u16),
    Network,
}

#[derive(Clone, Debug)]
pub(super) struct Call {
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

impl Call {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn json(&self) -> Json {
        serde_json::from_slice(&self.body).expect("the body is JSON")
    }
}

#[derive(Debug, Default)]
pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Outcome>>,
    calls: Mutex<Vec<Call>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: Vec<Outcome>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn call_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

fn decode(request: &PostRequest) -> Vec<u8> {
    let gzipped = request
        .headers
        .get("content-encoding")
        .is_some_and(|value| value == "gzip");
    if !gzipped {
        return request.body.clone();
    }
    let mut decoded = Vec::new();
    GzDecoder::new(request.body.as_slice())
        .read_to_end(&mut decoded)
        .expect("the gzip body decodes");
    decoded
}

fn record(transport: &ScriptedTransport, request: &PostRequest) {
    let headers = request
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().expect("header value is text").to_owned(),
            )
        })
        .collect();
    transport
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Call {
            url: request.url.clone(),
            headers,
            body: decode(request),
        });
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post(&self, request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        record(self, &request);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the scripted transport has a reply for every request");
        match reply {
            Outcome::Reply(status) => Ok(TransportResponse {
                status,
                body: Vec::new(),
            }),
            Outcome::Network => Err(TransportFailure::Request(TransportError::Rejected(
                HttpError::Timeout,
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FixedClock(pub(super) Duration);

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        self.0
    }
}

pub(super) fn build(
    environment: &Environment,
    init: MetricsInit,
    replies: Vec<Outcome>,
) -> (Arc<DatadogMetricsLogger>, Arc<ScriptedTransport>) {
    build_with(
        environment,
        init,
        replies,
        Arc::new(FixedClock(NOW_DURATION)),
        None,
    )
}

pub(super) fn build_with(
    environment: &Environment,
    init: MetricsInit,
    replies: Vec<Outcome>,
    clock: Arc<dyn Clock>,
    tasks: Option<TaskTracker>,
) -> (Arc<DatadogMetricsLogger>, Arc<ScriptedTransport>) {
    let transport = ScriptedTransport::new(replies);
    let concrete: Arc<ScriptedTransport> = Arc::clone(&transport);
    let transport_trait: Arc<dyn Transport> = concrete;
    let logger = DatadogMetricsLogger::new(
        environment,
        &init,
        MetricsDeps {
            transport: transport_trait,
            clock,
            tasks,
        },
    )
    .expect("the logger initializes");
    (logger, transport)
}

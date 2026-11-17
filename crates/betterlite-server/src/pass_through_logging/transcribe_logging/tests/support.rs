#![cfg(test)]

use std::fs::read_to_string;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redact::secret::Redactor;
use betterlite_transcribe::media_pricing::PricingError;
use betterlite_transcribe::policy::cost::TranscribeCostError;
use betterlite_transcribe::records::{StartedJob, TranscriptionJobRecord, TranscriptionMedia};
use chrono::{DateTime, FixedOffset};
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tempfile::NamedTempFile;
use tokio::sync::Semaphore;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use url::Url;

use super::super::{
    JobPricer, JobPricingFuture, JobPricingRequest, LivePricing, ScheduleInput,
    TranscribeHandlerInput, TranscribeLogger, TranscribeRates,
};

pub(super) const RESULT_TEXT: &str =
    r#"{"TranscriptionJob":{"TranscriptionJobStatus":"IN_PROGRESS"}}"#;
pub(super) const START_TARGET: &str = "Transcribe.StartTranscriptionJob";
pub(super) const OTHER_TARGET: &str = "Transcribe.DeleteTranscriptionJob";

// StandardLoggingPayload denies unknown fields and requires every key; optional ones are explicit nulls.
const PAYLOAD_TEMPLATE: &str = r#"{
    "classifier_input": null,
    "originating_request_masked": null,
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": null,
    "call_type": "pass_through_endpoint",
    "stream": null,
    "response_cost": 2.5,
    "cost_breakdown": null,
    "autorouter_savings": null,
    "autorouter_savings_estimate": null,
    "autorouter_baseline_observation": null,
    "response_cost_failure_debug_info": null,
    "zero_cost_diagnostic": null,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "custom_llm_provider": null,
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200,
    "endTime": 1704067201,
    "completionStartTime": 1704067201,
    "response_time": 1,
    "model_map_information": {},
    "model": "transcribe/StartTranscriptionJob",
    "model_id": null,
    "model_group": null,
    "api_base": "https://transcribe.us-east-1.amazonaws.com",
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
    "saved_cache_cost": 0,
    "request_tags": [],
    "request_model_access_groups": null,
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

fn at(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).expect("a valid timestamp")
}

pub(super) fn start_time() -> DateTime<FixedOffset> {
    at("2024-01-01T00:00:00+00:00")
}

pub(super) fn end_time() -> DateTime<FixedOffset> {
    at("2024-01-01T00:01:00+00:00")
}

pub(super) fn rate() -> Decimal {
    Decimal::new(2, 4)
}

pub(super) fn price() -> Decimal {
    Decimal::new(15, 3)
}

fn priced_cost() -> Decimal {
    Decimal::new(25, 1)
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON literal")
}

pub(super) fn json_map(text: &str) -> JsonMap {
    serde_json::from_str(text).expect("a JSON object")
}

pub(super) fn field<'value>(value: &'value Json, name: &str) -> &'value Json {
    let Json::Object(fields) = value else {
        panic!("the value is an object");
    };
    fields.get(name).expect("the field is present")
}

pub(super) fn entry<'kwargs>(
    kwargs: &'kwargs PassThroughLoggingKwargs,
    name: &str,
) -> &'kwargs Json {
    kwargs.entries.get(name).expect("the kwargs key is present")
}

fn kwargs(entries: &[(&str, Json)]) -> PassThroughLoggingKwargs {
    let mut collected = IndexMap::new();
    for (key, value) in entries {
        collected.insert((*key).to_owned(), value.clone());
    }
    PassThroughLoggingKwargs { entries: collected }
}

fn payload_fixture() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD_TEMPLATE).expect("the payload fixture parses")
}

fn header_value(text: &str) -> HeaderValue {
    HeaderValue::try_from(text).expect("a valid header value")
}

pub(super) fn headers_with(name: &'static str, value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(name, header_value(value));
    headers
}

pub(super) fn headers(target: &str) -> HeaderMap {
    headers_with("x-amz-target", target)
}

fn response_body() -> JsonMap {
    json_map(
        r#"{"TranscriptionJob":{"TranscriptionJobStatus":"COMPLETED","CreationTime":1704067200.0,"Media":{"MediaFileUri":"s3://bucket/job.mp4"}}}"#,
    )
}

fn request_body() -> JsonMap {
    json_map(r#"{"TranscriptionJobName":"job-1"}"#)
}

pub(super) fn job_record() -> TranscriptionJobRecord {
    TranscriptionJobRecord {
        transcription_job_status: Some("COMPLETED".to_owned()),
        creation_time: Some(1_704_067_200.0),
        media: Some(TranscriptionMedia {
            media_file_uri: Some("s3://bucket/job.mp4".to_owned()),
        }),
        tags: Vec::new(),
    }
}

struct NoopDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoopDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

fn logging_init() -> LoggingInit {
    LoggingInit {
        model: String::new(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: start_time(),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(false),
        litellm_trace_id: Some("trace-1".to_owned()),
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    }
}

fn logging_object() -> StreamLoggingObject {
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let logging = LoggingState::new(
        logging_init(),
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state");
    StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoopDispatch),
    }
}

pub(super) fn details(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .expect("the logging state is present")
}

pub(super) fn logged_model(logging_obj: &StreamLoggingObject) -> String {
    logging_obj
        .read(&mut |state| state.model.clone())
        .expect("the logging state is present")
}

pub(super) fn logged_provider(logging_obj: &StreamLoggingObject) -> Option<String> {
    logging_obj.read(&mut |state| state.custom_llm_provider.clone())?
}

#[derive(Clone, Debug)]
pub(super) struct BuildCall {
    pub(super) model: String,
    pub(super) provider: Option<String>,
    pub(super) details: IndexMap<String, Json>,
    pub(super) response: Json,
    pub(super) start_time: DateTime<FixedOffset>,
    pub(super) end_time: DateTime<FixedOffset>,
}

pub(super) struct FixedBuilder {
    payload: Option<StandardLoggingPayload>,
    calls: Mutex<Vec<BuildCall>>,
}

impl FixedBuilder {
    fn returning(payload: Option<StandardLoggingPayload>) -> Arc<Self> {
        Arc::new(Self {
            payload,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<BuildCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl StandardPayloadBuilder for FixedBuilder {
    fn build(
        &self,
        state: &LoggingState,
        response: &Json,
        start_time: DateTime<FixedOffset>,
        end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(BuildCall {
                model: state.model.clone(),
                provider: state.custom_llm_provider.clone(),
                details: state.model_call_details.clone(),
                response: response.clone(),
                start_time,
                end_time,
            });
        self.payload.clone()
    }
}

#[derive(Clone, Debug)]
pub(super) struct DispatchCall {
    pub(super) result: Json,
    pub(super) start_time: DateTime<FixedOffset>,
    pub(super) end_time: DateTime<FixedOffset>,
    pub(super) cache_hit: bool,
    pub(super) kwargs: PassThroughLoggingKwargs,
}

#[derive(Default)]
pub(super) struct RecordingDispatch {
    calls: Mutex<Vec<DispatchCall>>,
}

impl RecordingDispatch {
    pub(super) fn calls(&self) -> Vec<DispatchCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl StreamSuccessDispatch for RecordingDispatch {
    async fn dispatch_success(&self, call: StreamSuccessCall<'_>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(DispatchCall {
                result: call.result.response().clone(),
                start_time: call.start_time,
                end_time: call.end_time,
                cache_hit: call.cache_hit,
                kwargs: call.kwargs.clone(),
            });
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PricedJobCall {
    pub(super) job_name: String,
    pub(super) region: String,
    pub(super) cost_per_second: Decimal,
    pub(super) started_job: StartedJob,
}

pub(super) struct FixedPricer {
    cost: Option<Decimal>,
    calls: Mutex<Vec<PricedJobCall>>,
}

impl FixedPricer {
    pub(super) fn pricing(cost: Decimal) -> Arc<Self> {
        Arc::new(Self {
            cost: Some(cost),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn failing() -> Arc<Self> {
        Arc::new(Self {
            cost: None,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<PricedJobCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl JobPricer for FixedPricer {
    fn price_job<'job>(&'job self, request: JobPricingRequest<'job>) -> JobPricingFuture<'job> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(PricedJobCall {
                job_name: request.job_name.to_owned(),
                region: request.aws_region_name.to_owned(),
                cost_per_second: request.cost_per_second,
                started_job: request.started_job.clone(),
            });
        let cost = self.cost;
        Box::pin(async move { cost.ok_or(PricingError::Cost(TranscribeCostError::Overflow)) })
    }
}

struct FixedRates {
    cost_per_second: Option<Decimal>,
}

impl TranscribeRates for FixedRates {
    fn cost_per_second(&self) -> Option<Decimal> {
        self.cost_per_second
    }
}

pub(super) fn rates(cost: Option<Decimal>) -> Arc<dyn TranscribeRates> {
    Arc::new(FixedRates {
        cost_per_second: cost,
    })
}

pub(super) fn logger_with(
    pricer: Arc<FixedPricer>,
    cost_per_second: Option<Decimal>,
    max_pending: usize,
) -> TranscribeLogger {
    TranscribeLogger::new(
        pricer,
        rates(cost_per_second),
        max_pending,
        TaskTracker::new(),
    )
}

pub(super) struct Job {
    pub(super) logging_obj: Arc<StreamLoggingObject>,
    request_headers: HeaderMap,
    pub(super) request_url: Url,
    pub(super) response_body: Option<JsonMap>,
    pub(super) request_body: JsonMap,
    pub(super) log: Arc<RecordingDispatch>,
    kwargs: PassThroughLoggingKwargs,
    builder: Arc<FixedBuilder>,
    result: String,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    cache_hit: bool,
}

impl Job {
    pub(super) fn new() -> Self {
        Self {
            logging_obj: Arc::new(logging_object()),
            request_headers: headers(START_TARGET),
            request_url: Url::parse("https://transcribe.us-east-1.amazonaws.com/")
                .expect("a valid URL"),
            response_body: Some(response_body()),
            request_body: request_body(),
            log: Arc::new(RecordingDispatch::default()),
            kwargs: kwargs(&[("existing", Json::Bool(true))]),
            builder: FixedBuilder::returning(Some(payload_fixture())),
            result: RESULT_TEXT.to_owned(),
            start_time: start_time(),
            end_time: end_time(),
            cache_hit: true,
        }
    }

    fn input(&self) -> ScheduleInput<'_> {
        let log = Arc::clone(&self.log);
        let payload_builder = Arc::clone(&self.builder);
        ScheduleInput {
            request_headers: &self.request_headers,
            request_url: &self.request_url,
            response_body: self.response_body.as_ref(),
            logging_obj: Arc::clone(&self.logging_obj),
            url_route: "/transcribe",
            result: &self.result,
            start_time: self.start_time,
            end_time: self.end_time,
            cache_hit: self.cache_hit,
            request_body: &self.request_body,
            log,
            kwargs: self.kwargs.clone(),
            payload_builder,
        }
    }

    pub(super) async fn run(&self, logger: &TranscribeLogger) {
        logger
            .schedule_priced_job_logging(self.input())
            .await
            .expect("the scheduled task ran");
    }
}

pub(super) struct HandlerFixture {
    pub(super) logging_obj: StreamLoggingObject,
    pub(super) headers: HeaderMap,
    request_body: JsonMap,
    pub(super) kwargs: PassThroughLoggingKwargs,
    pub(super) builder: Arc<FixedBuilder>,
}

pub(super) fn handler_fixture() -> HandlerFixture {
    HandlerFixture {
        logging_obj: logging_object(),
        headers: headers(START_TARGET),
        request_body: request_body(),
        kwargs: kwargs(&[("existing", Json::Bool(true))]),
        builder: FixedBuilder::returning(Some(payload_fixture())),
    }
}

pub(super) fn failing_fixture() -> HandlerFixture {
    let mut fixture = handler_fixture();
    fixture.builder = FixedBuilder::returning(None);
    fixture
}

impl HandlerFixture {
    pub(super) fn input(&self) -> TranscribeHandlerInput<'_> {
        let response_cost = priced_cost();
        let payload_builder = self.builder.as_ref();
        let kwargs = self.kwargs.clone();
        TranscribeHandlerInput {
            logging_obj: &self.logging_obj,
            url_route: "/transcribe",
            result: RESULT_TEXT,
            start_time: start_time(),
            end_time: end_time(),
            cache_hit: false,
            request_headers: &self.headers,
            request_body: &self.request_body,
            response_cost,
            kwargs,
            payload_builder,
        }
    }
}

pub(super) fn capture_log() -> (NamedTempFile, DefaultGuard) {
    let log = NamedTempFile::new().expect("the log file opens");
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().expect("the log reopens")))
        .finish();
    (log, set_default(subscriber))
}

pub(super) fn log_text(log: &NamedTempFile) -> String {
    read_to_string(log.path()).expect("the log reads")
}

#[derive(Clone, Default)]
pub(super) struct RecordingLive {
    pub(super) slots: Arc<Mutex<Vec<Arc<Semaphore>>>>,
}

impl LivePricing for RecordingLive {
    fn price<'call>(
        &'call self,
        _request: JobPricingRequest<'call>,
        download_slots: Arc<Semaphore>,
    ) -> JobPricingFuture<'call> {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::clone(&download_slots));
        Box::pin(async { Ok(Decimal::ONE) })
    }
}

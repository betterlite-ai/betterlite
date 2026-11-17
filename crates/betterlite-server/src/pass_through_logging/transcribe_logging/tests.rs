#![cfg(test)]

use std::fs::read_to_string;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::media_relay_logging::spend::decimal_json;
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
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redact::secret::Redactor;
use betterlite_transcribe::media_pricing::PricingError;
use betterlite_transcribe::policy::cost::{TranscribeCostError, transcribe_max_job_cost};
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

use super::schedule::region_name;
use super::{
    JobPricer, JobPricingFuture, JobPricingRequest, LivePricing, MAX_PENDING_PRICING_JOBS,
    ScheduleInput, TranscribeHandlerInput, TranscribeLogger, TranscribeRates, is_priced_job_start,
    operation_from_target, transcribe_passthrough_handler,
};

const RESULT_TEXT: &str = r#"{"TranscriptionJob":{"TranscriptionJobStatus":"IN_PROGRESS"}}"#;
const START_TARGET: &str = "Transcribe.StartTranscriptionJob";
const OTHER_TARGET: &str = "Transcribe.DeleteTranscriptionJob";

const PAYLOAD_TEMPLATE: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "pass_through_endpoint",
    "response_cost": 2.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.0,
    "completionStartTime": 1704067201.0,
    "response_time": 1.0,
    "model_map_information": {},
    "model": "transcribe/StartTranscriptionJob",
    "api_base": "https://transcribe.us-east-1.amazonaws.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

fn at(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).expect("a valid timestamp")
}

fn start_time() -> DateTime<FixedOffset> {
    at("2024-01-01T00:00:00+00:00")
}

fn end_time() -> DateTime<FixedOffset> {
    at("2024-01-01T00:01:00+00:00")
}

fn rate() -> Decimal {
    Decimal::new(2, 4)
}

fn price() -> Decimal {
    Decimal::new(15, 3)
}

fn priced_cost() -> Decimal {
    Decimal::new(25, 1)
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON literal")
}

fn json_map(text: &str) -> JsonMap {
    serde_json::from_str(text).expect("a JSON object")
}

fn field<'value>(value: &'value Json, name: &str) -> &'value Json {
    let Json::Object(fields) = value else {
        panic!("the value is an object");
    };
    fields.get(name).expect("the field is present")
}

fn entry<'kwargs>(kwargs: &'kwargs PassThroughLoggingKwargs, name: &str) -> &'kwargs Json {
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

fn headers_with(name: &'static str, value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(name, header_value(value));
    headers
}

fn headers(target: &str) -> HeaderMap {
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

fn job_record() -> TranscriptionJobRecord {
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

fn details(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .expect("the logging state is present")
}

fn logged_model(logging_obj: &StreamLoggingObject) -> String {
    logging_obj
        .read(&mut |state| state.model.clone())
        .expect("the logging state is present")
}

fn logged_provider(logging_obj: &StreamLoggingObject) -> Option<String> {
    logging_obj
        .read(&mut |state| state.custom_llm_provider.clone())
        .expect("the logging state is present")
}

#[derive(Clone, Debug)]
struct BuildCall {
    model: String,
    provider: Option<String>,
    details: IndexMap<String, Json>,
    response: Json,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
}

struct FixedBuilder {
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

    fn calls(&self) -> Vec<BuildCall> {
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
struct DispatchCall {
    result: Json,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    cache_hit: bool,
    kwargs: PassThroughLoggingKwargs,
}

#[derive(Default)]
struct RecordingDispatch {
    calls: Mutex<Vec<DispatchCall>>,
}

impl RecordingDispatch {
    fn calls(&self) -> Vec<DispatchCall> {
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
struct PricedJobCall {
    job_name: String,
    region: String,
    cost_per_second: Decimal,
    started_job: StartedJob,
}

struct FixedPricer {
    cost: Option<Decimal>,
    calls: Mutex<Vec<PricedJobCall>>,
}

impl FixedPricer {
    fn pricing(cost: Decimal) -> Arc<Self> {
        Arc::new(Self {
            cost: Some(cost),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self {
            cost: None,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<PricedJobCall> {
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

fn rates(cost: Option<Decimal>) -> Arc<dyn TranscribeRates> {
    Arc::new(FixedRates {
        cost_per_second: cost,
    })
}

fn logger_with(
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

struct Job {
    logging_obj: Arc<StreamLoggingObject>,
    request_headers: HeaderMap,
    request_url: Url,
    response_body: Option<JsonMap>,
    request_body: JsonMap,
    log: Arc<RecordingDispatch>,
    kwargs: PassThroughLoggingKwargs,
    builder: Arc<FixedBuilder>,
    result: String,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    cache_hit: bool,
}

impl Job {
    fn new() -> Self {
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
            log: Arc::clone(&self.log),
            kwargs: self.kwargs.clone(),
            payload_builder: Arc::clone(&self.builder),
        }
    }

    async fn run(&self, logger: &TranscribeLogger) {
        logger
            .schedule_priced_job_logging(self.input())
            .await
            .expect("the scheduled task ran");
    }
}

struct HandlerFixture {
    logging_obj: StreamLoggingObject,
    headers: HeaderMap,
    request_body: JsonMap,
    kwargs: PassThroughLoggingKwargs,
    builder: Arc<FixedBuilder>,
}

fn handler_fixture() -> HandlerFixture {
    HandlerFixture {
        logging_obj: logging_object(),
        headers: headers(START_TARGET),
        request_body: request_body(),
        kwargs: kwargs(&[("existing", Json::Bool(true))]),
        builder: FixedBuilder::returning(Some(payload_fixture())),
    }
}

fn failing_fixture() -> HandlerFixture {
    let mut fixture = handler_fixture();
    fixture.builder = FixedBuilder::returning(None);
    fixture
}

impl HandlerFixture {
    fn input(&self) -> TranscribeHandlerInput<'_> {
        TranscribeHandlerInput {
            logging_obj: &self.logging_obj,
            url_route: "/transcribe",
            result: RESULT_TEXT,
            start_time: start_time(),
            end_time: end_time(),
            cache_hit: false,
            request_headers: &self.headers,
            request_body: &self.request_body,
            response_cost: priced_cost(),
            kwargs: self.kwargs.clone(),
            payload_builder: self.builder.as_ref(),
        }
    }
}

fn capture_log() -> (NamedTempFile, DefaultGuard) {
    let log = NamedTempFile::new().expect("the log file opens");
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().expect("the log reopens")))
        .finish();
    (log, set_default(subscriber))
}

fn log_text(log: &NamedTempFile) -> String {
    read_to_string(log.path()).expect("the log reads")
}

#[test]
fn operation_from_target_keeps_the_text_after_the_last_dot() {
    assert_eq!(
        operation_from_target(Some("Transcribe.StartTranscriptionJob")),
        "StartTranscriptionJob"
    );
}

#[test]
fn operation_from_target_cuts_nested_dots_at_the_last_one() {
    assert_eq!(
        operation_from_target(Some("com.amazon.transcribe.ListJobs")),
        "ListJobs"
    );
}

#[test]
fn operation_from_target_missing_target_is_empty() {
    assert_eq!(operation_from_target(None), "");
}

#[test]
fn operation_from_target_without_a_dot_comes_back_whole() {
    assert_eq!(
        operation_from_target(Some("StartTranscriptionJob")),
        "StartTranscriptionJob"
    );
}

#[test]
fn operation_from_target_trailing_dot_is_empty() {
    assert_eq!(operation_from_target(Some("Transcribe.")), "");
}

#[test]
fn operation_priced_start_reads_the_upstream_request_header() {
    assert!(is_priced_job_start(&headers(START_TARGET)));
}

#[test]
fn operation_priced_start_looks_up_the_header_case_insensitively() {
    assert!(is_priced_job_start(&headers_with(
        "X-Amz-Target",
        START_TARGET
    )));
}

#[test]
fn operation_priced_start_rejects_another_operation() {
    assert!(!is_priced_job_start(&headers(OTHER_TARGET)));
}

#[test]
fn operation_priced_start_rejects_a_missing_header() {
    assert!(!is_priced_job_start(&HeaderMap::new()));
}

#[test]
fn operation_priced_start_rejects_invalid_header_text() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-amz-target",
        HeaderValue::from_bytes(&[0xff]).expect("a raw value"),
    );
    assert!(!is_priced_job_start(&headers));
}

#[test]
fn operation_priced_start_matches_the_operation_alone() {
    assert_eq!(
        operation_from_target(Some(START_TARGET)),
        "StartTranscriptionJob"
    );
    assert_ne!(
        operation_from_target(Some(OTHER_TARGET)),
        "StartTranscriptionJob"
    );
}

#[test]
fn handler_returns_the_result_text_as_a_pass_through_response_object() {
    let fixture = handler_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    let expected = Json::Text(RESULT_TEXT.to_owned());
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::Standard(PassThroughResponseObject {
            response: expected,
        }))
    );
}

#[test]
fn handler_writes_the_model_provider_and_cost_into_the_kwargs() {
    let fixture = handler_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert_eq!(
        entry(&outcome.kwargs, "model"),
        &json("\"transcribe/StartTranscriptionJob\"")
    );
    assert_eq!(
        entry(&outcome.kwargs, "custom_llm_provider"),
        &json("\"transcribe\"")
    );
    assert_eq!(entry(&outcome.kwargs, "response_cost"), &json("2.5"));
    assert_eq!(entry(&outcome.kwargs, "existing"), &Json::Bool(true));
}

#[test]
fn handler_writes_the_model_call_details() {
    let fixture = handler_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert!(outcome.result.is_some());
    let written = details(&fixture.logging_obj);
    assert_eq!(
        written.get("model"),
        Some(&json("\"transcribe/StartTranscriptionJob\""))
    );
    assert_eq!(
        written.get("custom_llm_provider"),
        Some(&json("\"transcribe\""))
    );
    assert_eq!(written.get("response_cost"), Some(&json("2.5")));
    assert_eq!(
        logged_model(&fixture.logging_obj),
        "transcribe/StartTranscriptionJob"
    );
    assert_eq!(
        logged_provider(&fixture.logging_obj).as_deref(),
        Some("transcribe")
    );
}

#[test]
fn handler_builds_the_payload_after_the_model_call_details_write() {
    let fixture = handler_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert!(outcome.result.is_some());
    let calls = fixture.builder.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.model, "transcribe/StartTranscriptionJob");
    assert_eq!(call.provider.as_deref(), Some("transcribe"));
    assert_eq!(call.details.get("response_cost"), Some(&json("2.5")));
    assert_eq!(
        call.details.get("custom_llm_provider"),
        Some(&json("\"transcribe\""))
    );
    assert_eq!(call.start_time, start_time());
    assert_eq!(call.end_time, end_time());
    let response = Json::Text(RESULT_TEXT.to_owned());
    assert_eq!(field(&call.response, "response"), &response);
}

#[test]
fn handler_includes_the_standard_logging_object() {
    let fixture = handler_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    let payload = entry(&outcome.kwargs, "standard_logging_object");
    assert_eq!(field(payload, "id"), &json("\"chatcmpl-1\""));
    assert_eq!(field(payload, "status"), &json("\"success\""));
    assert_eq!(
        field(payload, "model"),
        &json("\"transcribe/StartTranscriptionJob\"")
    );
}

#[test]
fn handler_model_without_the_target_header_is_the_provider_and_a_slash() {
    let mut fixture = handler_fixture();
    fixture.headers = HeaderMap::new();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert_eq!(entry(&outcome.kwargs, "model"), &json("\"transcribe/\""));
}

#[test]
fn fallback_returns_the_original_kwargs() {
    let fixture = failing_fixture();
    let original = fixture.kwargs.clone();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert_eq!(outcome.kwargs, original);
}

#[test]
fn fallback_keeps_the_model_call_details_write() {
    let fixture = failing_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert!(outcome.result.is_some());
    let written = details(&fixture.logging_obj);
    assert_eq!(
        written.get("model"),
        Some(&json("\"transcribe/StartTranscriptionJob\""))
    );
    assert_eq!(
        written.get("custom_llm_provider"),
        Some(&json("\"transcribe\""))
    );
    assert_eq!(written.get("response_cost"), Some(&json("2.5")));
}

#[test]
fn fallback_still_returns_the_result_text() {
    let fixture = failing_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    let expected = Json::Text(RESULT_TEXT.to_owned());
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::Standard(PassThroughResponseObject {
            response: expected,
        }))
    );
}

#[test]
fn fallback_logs_the_error() {
    let (log, _guard) = capture_log();
    let fixture = failing_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert!(outcome.result.is_some());
    assert!(log_text(&log).contains(
        "Error in Amazon Transcribe passthrough logging handler: the standard logging payload could not be built"
    ));
}

#[test]
fn fallback_when_the_state_was_already_taken() {
    let fixture = handler_fixture();
    let taken = fixture
        .logging_obj
        .slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    assert!(taken.is_some());
    let original = fixture.kwargs.clone();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert_eq!(outcome.kwargs, original);
    assert!(fixture.builder.calls().is_empty());
}

#[tokio::test]
async fn schedule_prices_with_the_job_name_region_rate_and_started_job() {
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    let calls = pricer.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.job_name, "job-1");
    assert_eq!(call.region, "us-east-1");
    assert_eq!(call.cost_per_second, rate());
    assert_eq!(call.started_job, Some(job_record()));
}

#[tokio::test]
async fn schedule_region_is_the_second_host_label() {
    let mut job = Job::new();
    job.request_url =
        Url::parse("https://transcribe.cn-north-1.amazonaws.com.cn/").expect("a valid URL");
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert_eq!(pricer.calls()[0].region, "cn-north-1");
}

#[tokio::test]
async fn schedule_job_name_that_is_not_a_string_is_empty() {
    let mut job = Job::new();
    job.request_body = json_map(r#"{"TranscriptionJobName":7}"#);
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert_eq!(pricer.calls()[0].job_name, "");
}

#[tokio::test]
async fn schedule_missing_job_name_is_empty() {
    let mut job = Job::new();
    job.request_body = json_map("{}");
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert_eq!(pricer.calls()[0].job_name, "");
}

#[tokio::test]
async fn schedule_missing_response_body_has_no_started_job() {
    let mut job = Job::new();
    job.response_body = None;
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert_eq!(pricer.calls()[0].started_job, None);
}

#[tokio::test]
async fn schedule_dispatches_the_priced_cost_and_the_received_cache_hit() {
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    let calls = job.log.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert!(call.cache_hit);
    assert_eq!(entry(&call.kwargs, "response_cost"), &decimal_json(price()));
    assert_eq!(
        entry(&call.kwargs, "model"),
        &json("\"transcribe/StartTranscriptionJob\"")
    );
    assert_eq!(call.start_time, start_time());
    assert_eq!(call.end_time, end_time());
}

#[tokio::test]
async fn schedule_dispatches_the_handler_result_and_payload() {
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    let calls = job.log.calls();
    assert_eq!(calls.len(), 1);
    let expected = Json::Text(RESULT_TEXT.to_owned());
    assert_eq!(field(&calls[0].result, "response"), &expected);
    let payload = entry(&calls[0].kwargs, "standard_logging_object");
    assert_eq!(field(payload, "id"), &json("\"chatcmpl-1\""));
    assert_eq!(entry(&calls[0].kwargs, "existing"), &Json::Bool(true));
}

#[tokio::test]
async fn schedule_restores_the_logging_state_after_the_dispatch() {
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert_eq!(
        logged_model(&job.logging_obj),
        "transcribe/StartTranscriptionJob"
    );
}

#[tokio::test]
async fn failure_missing_rate_logs_and_dispatches_nothing() {
    let (log, _guard) = capture_log();
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), None, 10);
    job.run(&logger).await;
    assert!(job.log.calls().is_empty());
    assert!(pricer.calls().is_empty());
    assert!(
        log_text(&log).contains(
            "transcribe/StartTranscriptionJob left the model cost map, spend not recorded"
        )
    );
}

#[test]
fn failure_region_guards_reject_hosts_without_a_valid_second_label() {
    assert_eq!(region_name("transcribe"), None);
    assert_eq!(region_name("transcribe..amazonaws.com"), None);
    assert_eq!(region_name("transcribe.US-EAST-1.amazonaws.com"), None);
    assert_eq!(region_name("transcribe.us_east_1.amazonaws.com"), None);
    assert_eq!(
        region_name("transcribe.us-east-1.amazonaws.com"),
        Some("us-east-1")
    );
    assert_eq!(
        region_name("transcribe.cn-north-1.amazonaws.com.cn"),
        Some("cn-north-1")
    );
}

#[tokio::test]
async fn failure_region_without_a_second_label_dispatches_nothing() {
    let mut job = Job::new();
    job.request_url = Url::parse("https://transcribe/").expect("a valid URL");
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert!(job.log.calls().is_empty());
    assert!(pricer.calls().is_empty());
}

#[tokio::test]
async fn failure_pricer_error_dispatches_nothing() {
    let job = Job::new();
    let pricer = FixedPricer::failing();
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert!(job.log.calls().is_empty());
    assert_eq!(pricer.calls().len(), 1);
}

#[tokio::test]
async fn failure_taken_state_dispatches_nothing_and_ends_the_task() {
    let job = Job::new();
    let taken = job
        .logging_obj
        .slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    assert!(taken.is_some());
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert!(job.log.calls().is_empty());
    assert_eq!(pricer.calls().len(), 1);
}

#[tokio::test]
async fn limits_full_queue_charges_the_maximum_without_pricing() {
    let (log, _guard) = capture_log();
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 1);
    let held = Arc::clone(&logger.pending)
        .try_acquire_owned()
        .expect("a permit is free");
    job.run(&logger).await;
    assert!(pricer.calls().is_empty());
    let calls = job.log.calls();
    assert_eq!(calls.len(), 1);
    let maximum = transcribe_max_job_cost(rate()).expect("a valid maximum");
    assert_eq!(
        entry(&calls[0].kwargs, "response_cost"),
        &decimal_json(maximum)
    );
    assert!(
        log_text(&log).contains("Transcribe pricing queue is full, charging maximum for job job-1")
    );
    drop(held);
}

#[tokio::test]
async fn limits_permit_is_released_after_the_task_ends() {
    let first = Job::new();
    let second = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 1);
    first.run(&logger).await;
    second.run(&logger).await;
    assert_eq!(pricer.calls().len(), 2);
    assert_eq!(first.log.calls().len(), 1);
    assert_eq!(second.log.calls().len(), 1);
    assert_eq!(logger.pending.available_permits(), 1);
}

#[derive(Clone, Default)]
struct RecordingLive {
    slots: Arc<Mutex<Vec<Arc<Semaphore>>>>,
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

#[tokio::test]
async fn wiring_production_shares_one_download_semaphore_across_pricing_calls() {
    let live = RecordingLive::default();
    let logger =
        TranscribeLogger::production(live.clone(), rates(Some(rate())), TaskTracker::new());
    let started_job: StartedJob = None;
    let first = logger
        .job_pricer
        .price_job(JobPricingRequest {
            job_name: "job-1",
            aws_region_name: "us-east-1",
            cost_per_second: rate(),
            started_job: &started_job,
        })
        .await
        .expect("the production pricer prices");
    let second = logger
        .job_pricer
        .price_job(JobPricingRequest {
            job_name: "job-2",
            aws_region_name: "eu-west-1",
            cost_per_second: rate(),
            started_job: &started_job,
        })
        .await
        .expect("the production pricer prices");
    assert_eq!(first, Decimal::ONE);
    assert_eq!(second, Decimal::ONE);
    let slots = live
        .slots
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(slots.len(), 2);
    assert!(Arc::ptr_eq(&slots[0], &slots[1]));
    assert_eq!(slots[0].available_permits(), 1);
    assert_eq!(logger.pending.available_permits(), MAX_PENDING_PRICING_JOBS);
}

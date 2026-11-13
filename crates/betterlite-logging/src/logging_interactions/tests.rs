#![cfg(test)]

use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_interactions::polling::host::SettlementHost as _;
use betterlite_interactions::types::response::InteractionsApiResponse;
use betterlite_models::call_helpers::metadata::MetadataMap;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rstest::rstest;

use super::completion::{
    BillingError, InteractionLoggingCallbacks, InteractionLoggingObject, InteractionLoggingState,
};
use super::dedupe::reset_success_emission_dedupe;
use super::recognition::is_billable_interaction;
use super::reservation::{ReleaseFuture, ReservationRelease, reservation_from_params};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::exporter_options::redaction::ResponseStamp;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::types::{CallbackPolicy, DynamicCallbacks, RequestLoggingFlags};
use crate::logging_state::{LoggingInit, LoggingState};
use crate::logging_success::ports::{StandardPayloadBuilder, StreamAssembler, SuccessClock};
use crate::logging_success::types::{SuccessMode, SuccessResult};
use crate::registry::CallbackRegistry;
use crate::registry::Registration;
use crate::registry::identity::CallbackIdentity;
use crate::registry::phases::CallbackList;
use crate::registry::sink::CallbackSink;
use betterlite_redact::secret::Redactor;

const PAYLOAD: &str = r#"{
    "id": "interaction-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "create_interaction",
    "response_cost": 0.0,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gemini-2.5-flash",
    "api_base": "https://generativelanguage.googleapis.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": null,
    "response": {"id": "interaction-1", "status": "completed"}
}"#;

fn time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").expect("the fixture time parses")
}

fn params(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the fixture parameters parse")
}

fn logging_state(litellm_params: IndexMap<String, Json>) -> LoggingState {
    let mut correlation = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "gemini-2.5-flash".to_owned(),
            call_type: "create_interaction".to_owned(),
            start_time: time(),
            litellm_call_id: "call-1".to_owned(),
            function_id: "function-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: None,
            kwargs: IndexMap::new(),
            litellm_params,
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut correlation,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(16).expect("the redactor builds"),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("the logging state builds")
}

#[derive(Default)]
struct RecordingBuilder(Mutex<Vec<Json>>);

impl RecordingBuilder {
    fn seen(&self) -> Vec<Json> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl StandardPayloadBuilder for RecordingBuilder {
    fn build(
        &self,
        _state: &LoggingState,
        response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(response.clone());
        Some(serde_json::from_str(PAYLOAD).expect("the fixture payload parses"))
    }
}

struct NoStreams;

impl StreamAssembler for NoStreams {
    fn assemble(
        &self,
        _state: &LoggingState,
        _result: &SuccessResult,
        _mode: SuccessMode,
    ) -> Option<Json> {
        None
    }
}

struct FixedClock(DateTime<FixedOffset>);

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

#[derive(Default)]
struct RecordedEvents(Mutex<Vec<Arc<LoggerEvent>>>);

struct RecordingLogger {
    name: String,
    events: Arc<RecordedEvents>,
}

impl RecordingLogger {
    fn events(&self) -> usize {
        self.events
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[async_trait::async_trait]
impl CallbackLogger for RecordingLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.events
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the release failed")]
struct ReleaseFailure;

#[derive(Clone, Default)]
struct RecordingRelease {
    released: Arc<Mutex<Vec<MetadataMap>>>,
}

impl RecordingRelease {
    fn released(&self) -> Vec<MetadataMap> {
        self.released
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ReservationRelease for RecordingRelease {
    type Error = ReleaseFailure;

    fn release<'release>(
        &'release self,
        reservation: &'release MetadataMap,
    ) -> ReleaseFuture<'release, Self::Error> {
        self.released
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(reservation.clone());
        Box::pin(ready(Ok(())))
    }
}

struct Fixture {
    host: InteractionLoggingObject<RecordingRelease>,
    builder: Arc<RecordingBuilder>,
    logger: Arc<RecordingLogger>,
}

fn registry(logger: &Arc<RecordingLogger>) -> CallbackRegistry {
    let mut registry = CallbackRegistry::new(10);
    let identity = CallbackIdentity {
        class_name: logger.name.clone(),
        attributes: IndexMap::new(),
    };
    let logger_arc = Arc::clone(logger);
    let sink: Arc<dyn CallbackLogger> = logger_arc;
    assert_ne!(
        registry.register(
            identity,
            CallbackSink::Logger(sink),
            CallbackList::Callbacks
        ),
        Registration::SkippedCapacity
    );
    registry
}

fn fixture(state: LoggingState) -> Fixture {
    let builder = Arc::new(RecordingBuilder::default());
    let logger = Arc::new(RecordingLogger {
        name: "recorder".to_owned(),
        events: Arc::default(),
    });
    let builder_arc = Arc::clone(&builder);
    let payload_builder: Arc<dyn StandardPayloadBuilder> = builder_arc;
    let callbacks = InteractionLoggingCallbacks {
        registry: Arc::new(registry(&logger)),
        payload_builder,
        stream_assembler: Arc::new(NoStreams),
        clock: Arc::new(FixedClock(time())),
        failure_counters: Vec::new(),
        excluded_fields: Vec::new(),
        known_callback: Arc::new(|_name: &str| false),
        stamp: ResponseStamp {
            id: "interaction-1".to_owned(),
            created: 1,
        },
        privacy: Settings::default(),
    };
    Fixture {
        host: InteractionLoggingObject {
            state: Arc::new(Mutex::new(Some(InteractionLoggingState {
                logging: state,
                correlation: CorrelationContext::default(),
            }))),
            callbacks,
            release: RecordingRelease::default(),
        },
        builder,
        logger,
    }
}

fn completed_response() -> InteractionsApiResponse {
    let mut response: InteractionsApiResponse =
        serde_json::from_str(r#"{"id":"interaction-1","object":"interaction","status":"completed","usage":{"total_tokens":5}}"#)
            .expect("the interaction response parses");
    response.hidden_params = serde_json::from_str(
        r#"{"response_cost": 0.5, "model_id": "poll-deployment", "litellm_model_name": "poll/model", "cache_key": "keep"}"#,
    )
    .expect("the hidden parameters parse");
    response
}

fn state_with_spans(spans: &str) -> LoggingState {
    let parsed: Json = serde_json::from_str(spans).expect("the span fixture parses");
    let otel_internal = IndexMap::from([("spans_logged".to_owned(), parsed)]);
    let metadata = IndexMap::from([("_otel_internal".to_owned(), Json::Object(otel_internal))]);
    let litellm_params = IndexMap::from([("metadata".to_owned(), Json::Object(metadata))]);
    let mut state = logging_state(litellm_params);
    state
        .model_call_details
        .insert("has_logged_async_success".to_owned(), Json::Bool(true));
    state
}

fn state_with_dedupe_markers() -> LoggingState {
    state_with_spans(
        r#"{"[\"OpenTelemetry\",1,\"success\"]": true, "[\"OpenTelemetry\",1,\"guardrail\",\"pii\"]": true}"#,
    )
}

fn spans_logged(details: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let Some(Json::Object(held)) = details.get("litellm_params") else {
        panic!("the parameters are an object");
    };
    let Some(Json::Object(metadata)) = held.get("metadata") else {
        panic!("the metadata stays");
    };
    let Some(Json::Object(otel_internal)) = metadata.get("_otel_internal") else {
        panic!("the otel markers stay");
    };
    let Some(Json::Object(spans_logged)) = otel_internal.get("spans_logged") else {
        panic!("the span markers stay");
    };
    spans_logged.clone()
}

fn hidden_params(response: &Json) -> IndexMap<String, Json> {
    let Json::Object(fields) = response else {
        panic!("the completion result is a JSON object");
    };
    let Some(Json::Object(hidden)) = fields.get("_hidden_params") else {
        panic!("the kept hidden parameters survive");
    };
    hidden.clone()
}

fn retained_spans(host: &InteractionLoggingObject<RecordingRelease>) -> IndexMap<String, Json> {
    let held = host.state.lock().unwrap_or_else(PoisonError::into_inner);
    let state = held.as_ref().expect("the logging state is put back");
    let spans = spans_logged(&state.logging.model_call_details);
    drop(held);
    spans
}

#[tokio::test]
async fn completion_strips_poll_scoped_hidden_params_resets_dedupe_and_logs_once() {
    let fixture = fixture(state_with_dedupe_markers());
    fixture
        .host
        .log_background_completion(&completed_response())
        .await
        .expect("the settled interaction logs");
    assert_eq!(fixture.logger.events(), 1);
    let seen = fixture.builder.seen();
    assert_eq!(seen.len(), 1, "the handler builds one payload");
    let response = seen.first().expect("the handler receives the response");
    assert_eq!(
        hidden_params(response),
        params(r#"{"cache_key": "keep"}"#),
        "only the poll-scoped hidden parameters are dropped"
    );
    assert!(!retained_spans(&fixture.host).contains_key("[\"OpenTelemetry\",1,\"success\"]"));
}

#[tokio::test]
async fn completion_without_a_logging_state_fails_billing() {
    let mut fixture = fixture(state_with_dedupe_markers());
    fixture.host.state = Arc::new(Mutex::new(None));
    let error = fixture
        .host
        .log_background_completion(&completed_response())
        .await
        .expect_err("a taken logging state cannot log");
    assert!(matches!(error, BillingError::StateTaken));
    assert_eq!(fixture.logger.events(), 0);
}

#[rstest]
#[case::create_interaction("create_interaction", true, true)]
#[case::acreate_interaction("acreate_interaction", true, true)]
#[case::create("create", true, true)]
#[case::acreate("acreate", true, true)]
#[case::create_without_usage("create_interaction", false, false)]
#[case::sdk_create_without_usage("create", false, false)]
#[case::get("get_interaction", true, false)]
#[case::delete("delete_interaction", true, false)]
#[case::cancel("cancel_interaction", true, false)]
#[case::other("completion", true, false)]
fn is_billable_interaction_recognizes_only_creates_with_usage(
    #[case] call_type: &str,
    #[case] has_usage: bool,
    #[case] expected: bool,
) {
    assert_eq!(is_billable_interaction(call_type, has_usage), expected);
}

#[test]
fn dedupe_reset_keeps_every_scope_that_does_not_end_in_success() {
    let mut state = state_with_spans(
        r#"{"[\"OpenTelemetry\",1,\"success\"]": true,
            "[\"Langfuse\",2,\"success\"]": true,
            "[\"OpenTelemetry\",1,\"failure\"]": true,
            "[\"OpenTelemetry\",1,\"guardrail\",\"pii\"]": true,
            "[\"OpenTelemetry\",1,\"success\",2]": true,
            "plain-key": true,
            "[]": true}"#,
    );
    reset_success_emission_dedupe(&mut state);
    let spans = spans_logged(&state.model_call_details);
    let kept: Vec<&str> = spans.keys().map(String::as_str).collect();
    assert_eq!(
        kept,
        vec![
            "[\"OpenTelemetry\",1,\"failure\"]",
            "[\"OpenTelemetry\",1,\"guardrail\",\"pii\"]",
            "[\"OpenTelemetry\",1,\"success\",2]",
            "plain-key",
            "[]",
        ]
    );
}

#[test]
fn dedupe_reset_removes_the_async_success_marker() {
    let mut state = state_with_spans(
        r#"{"[\"OpenTelemetry\",1,\"success\"]": true,
            "[\"OpenTelemetry\",1,\"guardrail\",\"success\"]": true,
            "[\"OpenTelemetry\",1,\"guardrail\",\"pii\"]": true}"#,
    );
    reset_success_emission_dedupe(&mut state);
    assert!(
        !state
            .model_call_details
            .contains_key("has_logged_async_success")
    );
    let spans = spans_logged(&state.model_call_details);
    assert!(!spans.contains_key("[\"OpenTelemetry\",1,\"success\"]"));
    assert!(!spans.contains_key("[\"OpenTelemetry\",1,\"guardrail\",\"success\"]"));
    assert!(spans.contains_key("[\"OpenTelemetry\",1,\"guardrail\",\"pii\"]"));
}

#[rstest]
#[case::missing_spans("{}")]
#[case::parameters_not_an_object(r#"{"litellm_params": "not-a-map"}"#)]
#[case::metadata_not_an_object(r#"{"litellm_params": {"metadata": []}}"#)]
#[case::otel_not_an_object(r#"{"litellm_params": {"metadata": {"_otel_internal": 1}}}"#)]
#[case::spans_not_an_object(
    r#"{"litellm_params": {"metadata": {"_otel_internal": {"spans_logged": "done"}}}}"#
)]
#[case::spans_a_list(
    r#"{"litellm_params": {"metadata": {"_otel_internal": {"spans_logged": ["success"]}}}}"#
)]
fn dedupe_reset_tolerates_missing_or_non_map_levels(#[case] details: &str) {
    let mut state = logging_state(IndexMap::new());
    state.model_call_details.clear();
    let Json::Object(entries) = serde_json::from_str::<Json>(details).expect("the fixture parses")
    else {
        panic!("the fixture is an object");
    };
    state.model_call_details.extend(entries);
    state
        .model_call_details
        .insert("has_logged_async_success".to_owned(), Json::Bool(true));
    reset_success_emission_dedupe(&mut state);
    assert!(
        !state
            .model_call_details
            .contains_key("has_logged_async_success")
    );
}

#[test]
fn budget_reservation_takes_the_callers_copy_when_both_maps_carry_one_and_reads_only_maps() {
    let caller_only = params(r#"{"metadata": {"user_api_key_budget_reservation": {"spend": 2}}}"#);
    assert_eq!(
        reservation_from_params(&caller_only),
        Some(params(r#"{"spend": 2}"#))
    );
    let internal_only =
        params(r#"{"litellm_metadata": {"user_api_key_budget_reservation": {"reserved": true}}}"#);
    assert_eq!(
        reservation_from_params(&internal_only),
        Some(params(r#"{"reserved": true}"#))
    );
    let both = params(
        r#"{"metadata": {"user_api_key_budget_reservation": {"caller": 1}},
            "litellm_metadata": {"user_api_key_budget_reservation": {"internal": 2}}}"#,
    );
    // LiteLLM copies every user_api_key key of metadata over litellm_metadata: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L282-L293
    assert_eq!(
        reservation_from_params(&both),
        Some(params(r#"{"caller": 1}"#))
    );
    let internal_without_reservation = params(
        r#"{"metadata": {"user_api_key_budget_reservation": {"caller": 1}},
            "litellm_metadata": {"user": "someone"}}"#,
    );
    assert_eq!(
        reservation_from_params(&internal_without_reservation),
        Some(params(r#"{"caller": 1}"#))
    );
    let not_a_map = params(r#"{"metadata": {"user_api_key_budget_reservation": "held"}}"#);
    assert_eq!(reservation_from_params(&not_a_map), None);
    assert_eq!(reservation_from_params(&IndexMap::new()), None);
}

#[tokio::test]
async fn budget_reservation_reads_the_logging_state() {
    let mut state = logging_state(params(r#"{"metadata": {"user": "someone"}}"#));
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        Json::Object(params(
            r#"{"metadata": {"user_api_key_budget_reservation": {"spend": 3}}}"#,
        )),
    );
    let fixture = fixture(state);
    assert_eq!(
        fixture.host.budget_reservation(),
        Some(params(r#"{"spend": 3}"#))
    );
    let reservation = params(r#"{"spend": 3}"#);
    fixture
        .host
        .release_reservation(&reservation)
        .await
        .expect("the injected release port answers");
    assert_eq!(fixture.host.release.released(), vec![reservation]);
}

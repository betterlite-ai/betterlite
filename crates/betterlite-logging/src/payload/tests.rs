#![cfg(test)]

use std::io::{Error as IoError, Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_error::normalize::Normalizer;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::REDACTED_TEXT;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;
use crate::logging_failure::callbacks::FailureCallbacks;
use crate::logging_failure::handlers::SyncFailureJob;
use crate::logging_failure::{
    FailureCall, FailureClock, FailurePayloadInput, FailurePayloadSource, FailureRecord,
    FailureReport, SyncAdmission, SyncFailureExecutor,
};
use crate::logging_metadata::base64::Base64Truncator;
use crate::logging_metadata::exception::ExceptionFacts;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, Messages, RequestLoggingFlags,
};
use crate::logging_state::{LoggingInit, LoggingState};
use crate::logging_success::payload_state::standard_logging_object;
use crate::logging_success::ports::{StreamAssembler, SuccessClock};
use crate::logging_success::types::{SuccessMode, SuccessRequest, SuccessResult};
use crate::payload::builder::{PayloadBuilder, PayloadBuilderConfig};
use crate::payload::deps::{
    AbsentModelCostInformation, AbsentRequestAccessGroups, AutorouterAudit, PayloadClock,
    PayloadSettings, PayloadSources, SavedCacheCost,
};
use crate::payload::emit::{
    EmitError, EmitSettings, PayloadEmitter, emit_standard_logging_payload,
};
use crate::payload::lifecycle::{LoggingLifecycle, LoggingLifecycleConfig};
use crate::payload::redaction::redact_standard_payload;
use crate::payload::response::object_field;
use crate::registry::CallbackRegistry;

mod behaviors;
mod contracts;

pub(super) fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

pub(super) fn start() -> DateTime<FixedOffset> {
    time("2024-01-01T00:00:00+00:00")
}

pub(super) fn end() -> DateTime<FixedOffset> {
    time("2024-01-01T00:00:01+00:00")
}

pub(super) fn init(stream: Option<bool>) -> LoggingInit {
    LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "completion".to_owned(),
        start_time: start(),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream,
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

fn state(stream: Option<bool>) -> LoggingState {
    state_from(init(stream))
}

pub(super) fn state_from(configuration: LoggingInit) -> LoggingState {
    LoggingState::new(
        configuration,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(16).unwrap(),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .unwrap()
}

pub(super) fn chat_response() -> Json {
    serde_json::from_str(
        r#"{
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "model": "gpt-4o",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hello"}}],
            "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
        }"#,
    )
    .unwrap()
}

fn request(result: SuccessResult) -> SuccessRequest {
    SuccessRequest {
        result,
        start_time: Some(start()),
        end_time: Some(end()),
        cache_hit: Some(false),
        prefer_async_handlers: true,
        standard_logging_object: None,
        guardrail_snapshot: None,
    }
}

fn builder(privacy: Settings) -> PayloadBuilder {
    builder_with(
        PayloadSources::default(),
        PayloadSettings::default(),
        privacy,
    )
}

pub(super) fn builder_with(
    sources: PayloadSources,
    settings: PayloadSettings,
    privacy: Settings,
) -> PayloadBuilder {
    PayloadBuilder::new(PayloadBuilderConfig {
        sources: Arc::new(sources),
        settings,
        redactor: Arc::new(Redactor::new(16).unwrap()),
        normalizer: Arc::new(Normalizer::new().unwrap()),
        truncator: Base64Truncator::new(0).unwrap(),
        privacy,
    })
}

// names every source so a test shows which one it replaces and which stay absent
pub(super) fn sources_with(
    clock: Arc<dyn PayloadClock>,
    saved_cache_cost: Arc<dyn SavedCacheCost>,
    autorouter: Arc<dyn AutorouterAudit>,
) -> PayloadSources {
    PayloadSources {
        model_cost: Arc::new(AbsentModelCostInformation),
        saved_cache_cost,
        access_groups: Arc::new(AbsentRequestAccessGroups),
        autorouter,
        cold_storage: None,
        clock,
        responses_converter: PayloadSources::default().responses_converter,
    }
}

pub(super) fn payload_from(text: &str) -> StandardLoggingPayload {
    serde_json::from_str(text).unwrap()
}

#[derive(Default)]
struct Recorder {
    printed: Mutex<Vec<StandardLoggingPayload>>,
}

impl Recorder {
    fn count(&self) -> usize {
        self.printed.lock().unwrap().len()
    }

    fn only(&self) -> StandardLoggingPayload {
        let printed = self.printed.lock().unwrap();
        assert_eq!(printed.len(), 1, "one terminal payload is printed");
        printed[0].clone()
    }
}

impl PayloadEmitter for Recorder {
    fn emit(&self, payload: &StandardLoggingPayload) -> Result<(), EmitError> {
        self.printed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload.clone());
        Ok(())
    }
}

struct FixedAssembler;

impl StreamAssembler for FixedAssembler {
    fn assemble(
        &self,
        state: &LoggingState,
        result: &SuccessResult,
        _mode: SuccessMode,
    ) -> Option<Json> {
        if state.stream != Some(true) {
            return None;
        }
        match result {
            SuccessResult::Chunk(_) => None,
            SuccessResult::Chat(_) | SuccessResult::Embeddings(_) | SuccessResult::Other(_) => {
                Some(result.response().clone())
            }
        }
    }
}

struct SuccessClockAt(DateTime<FixedOffset>);

impl SuccessClock for SuccessClockAt {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

struct FailureClockAt(DateTime<FixedOffset>);

impl FailureClock for FailureClockAt {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

struct RejectingExecutor;

impl SyncFailureExecutor for RejectingExecutor {
    fn enqueue_sync_failure(&self, _job: SyncFailureJob) -> SyncAdmission {
        SyncAdmission::Rejected
    }
}

fn lifecycle(builder: PayloadBuilder, emitter: Arc<Recorder>) -> LoggingLifecycle {
    LoggingLifecycle::new(LoggingLifecycleConfig {
        registry: Arc::new(CallbackRegistry::new(10)),
        payload: builder,
        stream_assembler: Arc::new(FixedAssembler),
        success_clock: Arc::new(SuccessClockAt(end())),
        failure_clock: Arc::new(FailureClockAt(end())),
        executor: Arc::new(RejectingExecutor),
        baseline: None,
        flush: None,
        special_listeners: Vec::new(),
        counters: Vec::<Arc<dyn CallbackFailureCounter + Send + Sync>>::new(),
        excluded_fields: Vec::new(),
        known_callback: Arc::new(|_name: &str| false),
        emitter,
    })
}

fn redacted_response(response: Json) -> Option<Json> {
    let mut payload = payload_from(BASE_PAYLOAD);
    payload.response = Some(response);
    redact_standard_payload(&payload).response
}

pub(super) const BASE_PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "",
    "call_type": "completion",
    "response_cost": 0.5,
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
    "model": "gpt-4o",
    "api_base": "",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "hello"}],
    "response": "raw-response"
}"#;

#[derive(Clone, Copy, Debug)]
enum Behavior {
    EmitDisabled,
    EmitEnabled,
    EmitEnvValues,
    EmitWriteFailure,
    DecimalSeconds,
    RedactionMessages,
    RedactionPlaceholders,
    RedactionChoiceBranches,
    RedactionOutputBranches,
    RedactionCopiesInput,
    RedactionKeepsAbsentResponse,
    PrivacyRedactsPayload,
    LifecycleOnce,
    LifecycleAfterCancel,
    LifecycleAfterFailure,
    StreamChunkWaits,
    StreamCompletePrintsOnce,
    FailurePayload,
}

#[rstest]
#[case::emit_disabled_prints_nothing(Behavior::EmitDisabled)]
#[case::emit_enabled_writes_one_json_line(Behavior::EmitEnabled)]
#[case::emit_env_values_follow_os_getenv(Behavior::EmitEnvValues)]
#[case::emit_reports_a_failed_write(Behavior::EmitWriteFailure)]
#[case::decimal_seconds_keep_a_fraction(Behavior::DecimalSeconds)]
#[case::redaction_replaces_messages(Behavior::RedactionMessages)]
#[case::redaction_placeholders_response_shapes(Behavior::RedactionPlaceholders)]
#[case::redaction_redacts_delta_and_choice_entries(Behavior::RedactionChoiceBranches)]
#[case::redaction_redacts_output_items(Behavior::RedactionOutputBranches)]
#[case::redaction_copies_the_payload(Behavior::RedactionCopiesInput)]
#[case::redaction_keeps_an_absent_response(Behavior::RedactionKeepsAbsentResponse)]
#[case::privacy_policy_redacts_the_logged_messages(Behavior::PrivacyRedactsPayload)]
#[case::lifecycle_prints_one_terminal_payload(Behavior::LifecycleOnce)]
#[case::lifecycle_stops_after_cancel(Behavior::LifecycleAfterCancel)]
#[case::lifecycle_stops_after_failure(Behavior::LifecycleAfterFailure)]
#[case::stream_chunk_prints_nothing(Behavior::StreamChunkWaits)]
#[case::stream_complete_prints_once(Behavior::StreamCompletePrintsOnce)]
#[case::failure_payload_reports_the_error(Behavior::FailurePayload)]
#[tokio::test]
async fn logging_payloads_logging_payload_tests_behavior(#[case] behavior: Behavior) {
    match behavior {
        Behavior::EmitDisabled => {
            let payload = payload_from(BASE_PAYLOAD);
            let mut sink = Vec::new();
            emit_standard_logging_payload(&payload, &EmitSettings::default(), &mut sink).unwrap();
            assert!(sink.is_empty());
        }
        Behavior::EmitEnabled => {
            let payload = payload_from(BASE_PAYLOAD);
            let settings = EmitSettings::from_env(Some("1"));
            let mut sink = Vec::new();
            emit_standard_logging_payload(&payload, &settings, &mut sink).unwrap();
            let line = sink
                .strip_suffix(b"\n")
                .expect("the emitted line ends with a newline");
            let printed: Json = serde_json::from_slice(line).unwrap();
            assert_eq!(
                serde_json::to_string(&printed).unwrap(),
                serde_json::to_string(&payload).unwrap()
            );
        }
        Behavior::EmitEnvValues => {
            let payload = payload_from(BASE_PAYLOAD);
            let mut zero = Vec::new();
            emit_standard_logging_payload(&payload, &EmitSettings::from_env(Some("0")), &mut zero)
                .unwrap();
            assert!(!zero.is_empty());
            let mut empty = Vec::new();
            emit_standard_logging_payload(&payload, &EmitSettings::from_env(Some("")), &mut empty)
                .unwrap();
            assert!(empty.is_empty());
            let mut absent = Vec::new();
            emit_standard_logging_payload(&payload, &EmitSettings::from_env(None), &mut absent)
                .unwrap();
            assert!(absent.is_empty());
        }
        Behavior::EmitWriteFailure => {
            let payload = payload_from(BASE_PAYLOAD);
            let mut sink = FailingSink;
            let settings = EmitSettings::from_env(Some("1"));
            assert!(matches!(
                emit_standard_logging_payload(&payload, &settings, &mut sink),
                Err(EmitError::Write { .. })
            ));
        }
        Behavior::DecimalSeconds => {
            let recorder = Arc::new(Recorder::default());
            let mut state = state(None);
            let mut correlation = CorrelationContext::default();
            let lifecycle = lifecycle(builder(Settings::default()), Arc::clone(&recorder));
            let result = SuccessResult::Chat(chat_response());
            lifecycle
                .success(&mut state, &mut correlation, &request(result))
                .await;
            let payload = recorder.only();
            assert_eq!(payload.start_time.to_string(), "1704067200.0");
            assert_eq!(payload.end_time.to_string(), "1704067201.0");
            assert_eq!(payload.response_time.to_string(), "1.0");
            assert_eq!(payload.status, LoggingStatus::Success);
        }
        Behavior::RedactionMessages => {
            let payload = payload_from(BASE_PAYLOAD);
            let redacted = redact_standard_payload(&payload);
            let messages = redacted.messages.unwrap();
            let Json::List(items) = messages else {
                panic!("messages must be a list");
            };
            assert_eq!(items.len(), 1);
            let Json::Object(fields) = &items[0] else {
                panic!("a message is an object");
            };
            assert_eq!(
                fields.get("content"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
            assert_eq!(fields.get("role"), Some(&Json::Text("user".to_owned())));
            assert!(redacted.classifier_input.is_none());
            assert!(redacted.originating_request_masked.is_none());
        }
        Behavior::RedactionPlaceholders => {
            let placeholder = Json::Object(IndexMap::from([(
                "text".to_owned(),
                Json::Text(REDACTED_TEXT.to_owned()),
            )]));
            assert_eq!(
                redacted_response(Json::Text("secret".to_owned())),
                Some(Json::Text(REDACTED_TEXT.to_owned()))
            );
            assert_eq!(redacted_response(Json::Null), Some(Json::Null));
            assert_eq!(
                redacted_response(Json::Bool(true)),
                Some(placeholder.clone())
            );
            assert_eq!(
                redacted_response(Json::Number(Number::from(7_i64))),
                Some(placeholder.clone())
            );
            assert_eq!(
                redacted_response(Json::List(Vec::new())),
                Some(placeholder.clone())
            );
            assert_eq!(
                redacted_response(Json::Object(IndexMap::new())),
                Some(placeholder)
            );
        }
        Behavior::RedactionChoiceBranches => {
            let mut payload = payload_from(BASE_PAYLOAD);
            payload.response = Some(
                serde_json::from_str(
                    r#"{
                        "choices": [
                            {"delta": {"content": "secret", "reasoning_content": "hidden",
                                "thinking_blocks": [{"text": "block"}], "audio": {"id": "a"},
                                "tool_calls": [{"function": {"name": "f", "arguments": "{}"}}]}},
                            {"text": "choice text"},
                            {"message": {"content": "hello", "reasoning_content": "hidden",
                                "thinking_blocks": [], "audio": {"id": "a"},
                                "function_call": {"name": "f", "arguments": "{}"}}}
                        ]
                    }"#,
                )
                .unwrap(),
            );
            let Json::Object(fields) = redact_standard_payload(&payload).response.unwrap() else {
                panic!("an object response stays an object");
            };
            let Json::List(choices) = fields.get("choices").unwrap() else {
                panic!("choices stay a list");
            };
            let Json::Object(delta) = object_field(&choices[0], "delta").unwrap() else {
                panic!("a delta is an object");
            };
            assert_eq!(
                delta.get("content"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
            assert_eq!(
                delta.get("reasoning_content"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
            assert_eq!(delta.get("thinking_blocks"), Some(&Json::Null));
            assert_eq!(delta.get("audio"), Some(&Json::Null));
            let Json::List(calls) = delta.get("tool_calls").unwrap() else {
                panic!("tool calls stay a list");
            };
            let function = object_field(&calls[0], "function").unwrap();
            assert_eq!(
                object_field(function, "arguments"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
            assert_eq!(
                object_field(&choices[1], "text"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
            let Json::Object(message) = object_field(&choices[2], "message").unwrap() else {
                panic!("a message is an object");
            };
            assert_eq!(
                message.get("reasoning_content"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
            assert_eq!(message.get("thinking_blocks"), Some(&Json::Null));
            assert_eq!(message.get("audio"), Some(&Json::Null));
            let function_call = message.get("function_call").unwrap();
            assert_eq!(
                object_field(function_call, "arguments"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
        }
        Behavior::RedactionOutputBranches => {
            let mut payload = payload_from(BASE_PAYLOAD);
            payload.response = Some(
                serde_json::from_str(
                    r#"{
                        "output": [
                            {"type": "message", "text": "item text",
                                "content": [{"type": "output_text", "text": "part", "refusal": "no"}]},
                            {"type": "custom_tool_call", "input": "secret input"}
                        ],
                        "vertex_ai_safety_ratings": [{"blocked": false}]
                    }"#,
                )
                .unwrap(),
            );
            let Json::Object(fields) = redact_standard_payload(&payload).response.unwrap() else {
                panic!("an object response stays an object");
            };
            assert_eq!(
                fields.get("vertex_ai_safety_ratings"),
                Some(&Json::List(Vec::new()))
            );
            let Json::List(items) = fields.get("output").unwrap() else {
                panic!("output stays a list");
            };
            assert_eq!(
                object_field(&items[0], "text"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
            let Json::List(content) = object_field(&items[0], "content").unwrap() else {
                panic!("content stays a list");
            };
            assert_eq!(
                object_field(&content[0], "text"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
            assert_eq!(
                object_field(&content[0], "refusal"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
            assert_eq!(
                object_field(&items[1], "input"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
        }
        Behavior::RedactionCopiesInput => {
            let payload = payload_from(BASE_PAYLOAD);
            let redacted = redact_standard_payload(&payload);
            assert_eq!(
                payload.messages,
                Some(serde_json::from_str(r#"[{"role":"user","content":"hello"}]"#).unwrap())
            );
            assert_ne!(
                serde_json::to_value(&payload).unwrap(),
                serde_json::to_value(&redacted).unwrap()
            );
        }
        Behavior::RedactionKeepsAbsentResponse => {
            let mut payload = payload_from(BASE_PAYLOAD);
            payload.response = None;
            assert!(redact_standard_payload(&payload).response.is_none());
            payload.messages = None;
            assert!(redact_standard_payload(&payload).messages.is_none());
        }
        Behavior::PrivacyRedactsPayload => {
            let mut configuration = init(None);
            configuration.messages = Some(Messages::Values(vec![Json::Object(IndexMap::from([
                ("role".to_owned(), Json::Text("user".to_owned())),
                ("content".to_owned(), Json::Text("hello".to_owned())),
            ]))]));
            let mut state = state_from(configuration);
            state.model_call_details.insert(
                "system".to_owned(),
                Json::Text("log the system prompt".to_owned()),
            );
            let mut correlation = CorrelationContext::default();
            let privacy = Settings {
                turn_off_message_logging: true,
                redact_user_api_key_info: false,
            };
            let recorder = Arc::new(Recorder::default());
            let lifecycle = lifecycle(builder(privacy), Arc::clone(&recorder));
            lifecycle
                .success(
                    &mut state,
                    &mut correlation,
                    &request(SuccessResult::Chat(chat_response())),
                )
                .await;
            let payload = recorder.only();
            let Json::List(messages) = payload.messages.unwrap() else {
                panic!("messages stay a list");
            };
            assert_eq!(messages.len(), 1);
            assert_eq!(
                object_field(&messages[0], "content"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
            let response = payload.response.unwrap();
            let Json::List(choices) = object_field(&response, "choices").unwrap() else {
                panic!("choices stay a list");
            };
            let message = object_field(&choices[0], "message").unwrap();
            assert_eq!(
                object_field(message, "content"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
        }
        Behavior::LifecycleOnce => {
            let recorder = Arc::new(Recorder::default());
            let mut state = state(None);
            let mut correlation = CorrelationContext::default();
            let lifecycle = lifecycle(builder(Settings::default()), Arc::clone(&recorder));
            let result = SuccessResult::Chat(chat_response());
            lifecycle
                .success(&mut state, &mut correlation, &request(result.clone()))
                .await;
            lifecycle
                .success(&mut state, &mut correlation, &request(result))
                .await;
            let payload = recorder.only();
            assert_eq!(payload.id, "chatcmpl-1");
            assert!(matches!(standard_logging_object(&state), Ok(Some(_))));
        }
        Behavior::LifecycleAfterCancel => {
            let recorder = Arc::new(Recorder::default());
            let mut state = state(None);
            let mut correlation = CorrelationContext::default();
            let lifecycle = lifecycle(builder(Settings::default()), Arc::clone(&recorder));
            lifecycle.cancel(&mut state);
            lifecycle
                .success(
                    &mut state,
                    &mut correlation,
                    &request(SuccessResult::Chat(chat_response())),
                )
                .await;
            assert_eq!(recorder.count(), 0);
            assert!(matches!(standard_logging_object(&state), Ok(None)));
        }
        Behavior::LifecycleAfterFailure => {
            let recorder = Arc::new(Recorder::default());
            let mut state = state(None);
            let mut record = FailureRecord::default();
            let mut correlation = CorrelationContext::default();
            let lifecycle = lifecycle(builder(Settings::default()), Arc::clone(&recorder));
            let report = FailureReport {
                exception: Arc::new(ExceptionFacts::default()),
                traceback: String::new(),
                start_time: Some(start()),
                end_time: Some(end()),
            };
            let registry = CallbackRegistry::new(10);
            let callbacks = FailureCallbacks {
                registry: &registry,
                dynamic: None,
            };
            let mut call = FailureCall {
                state: &mut state,
                record: &mut record,
                context: &mut correlation,
            };
            lifecycle
                .failure(&mut call, &report, &callbacks, true)
                .await;
            let mut success_correlation = CorrelationContext::default();
            lifecycle
                .success(
                    &mut state,
                    &mut success_correlation,
                    &request(SuccessResult::Chat(chat_response())),
                )
                .await;
            assert_eq!(recorder.count(), 0);
        }
        Behavior::StreamChunkWaits => {
            let recorder = Arc::new(Recorder::default());
            let mut state = state(Some(true));
            let mut correlation = CorrelationContext::default();
            let lifecycle = lifecycle(builder(Settings::default()), Arc::clone(&recorder));
            let chunk = SuccessResult::Chunk(chat_response());
            lifecycle
                .success(&mut state, &mut correlation, &request(chunk))
                .await;
            assert_eq!(recorder.count(), 0);
        }
        Behavior::StreamCompletePrintsOnce => {
            let recorder = Arc::new(Recorder::default());
            let mut state = state(Some(true));
            let mut correlation = CorrelationContext::default();
            let lifecycle = lifecycle(builder(Settings::default()), Arc::clone(&recorder));
            let chunk = SuccessResult::Chunk(chat_response());
            lifecycle
                .success(&mut state, &mut correlation, &request(chunk))
                .await;
            let complete = SuccessResult::Chat(chat_response());
            lifecycle
                .success(&mut state, &mut correlation, &request(complete))
                .await;
            let payload = recorder.only();
            assert_eq!(payload.stream, Some(true));
        }
        Behavior::FailurePayload => {
            let exception: ExceptionFacts =
                serde_json::from_str(r#"{"message": "server exploded", "llm_provider": "openai"}"#)
                    .unwrap();
            let state = state(None);
            let payload = FailurePayloadSource::build(
                &builder(Settings::default()),
                FailurePayloadInput {
                    state: &state,
                    exception: &exception,
                    start_time: start(),
                    end_time: end(),
                    error_str: "server exploded",
                },
            )
            .unwrap();
            assert_eq!(payload.status, LoggingStatus::Failure);
            assert_eq!(payload.error_str.as_deref(), Some("server exploded"));
            assert_eq!(payload.total_tokens, 0);
            assert_eq!(payload.response_cost.to_string(), "0.0");
            assert_eq!(payload.response, Some(Json::Object(IndexMap::new())));
        }
    }
}

struct FailingSink;

impl Write for FailingSink {
    fn write(&mut self, _buffer: &[u8]) -> IoResult<usize> {
        Err(IoError::other("sink is closed"))
    }

    fn flush(&mut self) -> IoResult<()> {
        Err(IoError::other("sink is closed"))
    }
}

#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_error::normalize::Normalizer;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rstest::rstest;

use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::ResponseStamp;
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
use crate::payload::deps::{PayloadSettings, PayloadSources};
use crate::payload::emit::{
    EmitError, EmitSettings, PayloadEmitter, emit_standard_logging_payload,
};
use crate::payload::lifecycle::{LoggingLifecycle, LoggingLifecycleConfig};
use crate::payload::redaction::{REDACTED_TEXT, redacted_standard_logging_payload};
use crate::payload::response::object_field;
use crate::registry::CallbackRegistry;

fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

fn start() -> DateTime<FixedOffset> {
    time("2024-01-01T00:00:00+00:00")
}

fn end() -> DateTime<FixedOffset> {
    time("2024-01-01T00:00:01+00:00")
}

fn init(stream: Option<bool>) -> LoggingInit {
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

fn state_from(configuration: LoggingInit) -> LoggingState {
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

fn chat_response() -> Json {
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
    PayloadBuilder::new(PayloadBuilderConfig {
        sources: Arc::new(PayloadSources::default()),
        settings: PayloadSettings::default(),
        redactor: Arc::new(Redactor::new(16).unwrap()),
        normalizer: Arc::new(Normalizer::new().unwrap()),
        truncator: Base64Truncator::new(0).unwrap(),
        privacy,
    })
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
        self.printed.lock().unwrap().push(payload.clone());
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
        stamp: ResponseStamp {
            id: "chatcmpl-1".to_owned(),
            created: 1_704_067_200,
        },
        emitter,
    })
}

fn payload_from(text: &str) -> StandardLoggingPayload {
    serde_json::from_str(text).unwrap()
}

const BASE_PAYLOAD: &str = r#"{
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
    EmitWriteFailure,
    DecimalSeconds,
    RedactionMessages,
    RedactionPlaceholders,
    RedactionChoices,
    RedactionOutput,
    RedactionLeavesInput,
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
#[case::emit_reports_a_failed_write(Behavior::EmitWriteFailure)]
#[case::decimal_seconds_keep_a_fraction(Behavior::DecimalSeconds)]
#[case::redaction_replaces_messages(Behavior::RedactionMessages)]
#[case::redaction_placeholders_response_shapes(Behavior::RedactionPlaceholders)]
#[case::redaction_scrubs_choices_and_vertex(Behavior::RedactionChoices)]
#[case::redaction_output_items_and_vertex(Behavior::RedactionOutput)]
#[case::redaction_copies_the_payload(Behavior::RedactionLeavesInput)]
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
            assert!(sink.ends_with(b"\n"));
            let printed: Json = serde_json::from_slice(&sink[..sink.len() - 1]).unwrap();
            assert_eq!(printed, serde_json::to_value(&payload).unwrap());
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
            let redacted = redacted_standard_logging_payload(&payload);
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
                Some(&Json::Text("redacted-by-litellm".to_owned()))
            );
            assert_eq!(fields.get("role"), Some(&Json::Text("user".to_owned())));
            assert!(redacted.classifier_input.is_none());
            assert!(redacted.originating_request_masked.is_none());
        }
        Behavior::RedactionPlaceholders => {
            for response in [
                Json::Text("secret".to_owned()),
                Json::Null,
                Json::Bool(true),
                Json::Number(serde_json::Number::from(7)),
                Json::List(Vec::new()),
                Json::Object(IndexMap::new()),
            ] {
                let mut payload = payload_from(BASE_PAYLOAD);
                payload.response = Some(response);
                if matches!(payload.response, Some(Json::Text(_))) {
                    assert_eq!(
                        redacted_standard_logging_payload(&payload).response,
                        Some(Json::Text("redacted-by-litellm".to_owned()))
                    );
                    continue;
                }
                assert_eq!(
                    redacted_standard_logging_payload(&payload).response,
                    Some(Json::Object(IndexMap::from([(
                        "text".to_owned(),
                        Json::Text("redacted-by-litellm".to_owned())
                    )])))
                );
            }
        }
        Behavior::RedactionChoices => {
            let mut payload = payload_from(BASE_PAYLOAD);
            payload.response = Some(
                serde_json::from_str(
                    r#"{
                        "choices": [{"message": {"role": "assistant", "content": "secret",
                            "tool_calls": [{"function": {"name": "f", "arguments": "{}"}}]}}],
                        "vertex_ai_grounding_metadata": [{"secret": true}],
                        "_hidden_params": {"vertex_ai_safety_ratings": [{"blocked": false}]}
                    }"#,
                )
                .unwrap(),
            );
            let redacted = redacted_standard_logging_payload(&payload);
            let Json::Object(fields) = redacted.response.unwrap() else {
                panic!("an object response stays an object");
            };
            assert_eq!(
                fields.get("vertex_ai_grounding_metadata"),
                Some(&Json::List(Vec::new()))
            );
            let Json::Object(hidden) = fields.get("_hidden_params").unwrap() else {
                panic!("hidden params stay an object");
            };
            assert!(!hidden.contains_key("vertex_ai_safety_ratings"));
            let Json::List(choices) = fields.get("choices").unwrap() else {
                panic!("choices stay a list");
            };
            let Json::Object(choice) = &choices[0] else {
                panic!("a choice is an object");
            };
            let Json::Object(message) = choice.get("message").unwrap() else {
                panic!("a message is an object");
            };
            assert_eq!(
                message.get("content"),
                Some(&Json::Text("redacted-by-litellm".to_owned()))
            );
        }
        Behavior::RedactionOutput => {
            let mut payload = payload_from(BASE_PAYLOAD);
            payload.response = Some(
                serde_json::from_str(
                    r#"{
                        "output": [
                            {"type": "message", "content": [{"type": "output_text", "text": "secret"}]},
                            {"type": "function_call", "arguments": "{}"},
                            {"type": "reasoning", "summary": [{"text": "secret"}]}
                        ],
                        "vertex_ai_citation_metadata": [{"secret": true}]
                    }"#,
                )
                .unwrap(),
            );
            let redacted = redacted_standard_logging_payload(&payload);
            let Json::Object(fields) = redacted.response.unwrap() else {
                panic!("an object response stays an object");
            };
            assert_eq!(
                fields.get("vertex_ai_citation_metadata"),
                Some(&Json::List(Vec::new()))
            );
            let Json::List(items) = fields.get("output").unwrap() else {
                panic!("output stays a list");
            };
            let Json::Object(call) = &items[1] else {
                panic!("an output item is an object");
            };
            assert_eq!(
                call.get("arguments"),
                Some(&Json::Text("redacted-by-litellm".to_owned()))
            );
            let Json::Object(reasoning) = &items[2] else {
                panic!("an output item is an object");
            };
            let Json::List(summary) = reasoning.get("summary").unwrap() else {
                panic!("summary stays a list");
            };
            let Json::Object(entry) = &summary[0] else {
                panic!("a summary entry is an object");
            };
            assert_eq!(
                entry.get("text"),
                Some(&Json::Text("redacted-by-litellm".to_owned()))
            );
        }
        Behavior::RedactionLeavesInput => {
            let payload = payload_from(BASE_PAYLOAD);
            let redacted = redacted_standard_logging_payload(&payload);
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
            assert!(
                redacted_standard_logging_payload(&payload)
                    .response
                    .is_none()
            );
            payload.messages = None;
            assert!(
                redacted_standard_logging_payload(&payload)
                    .messages
                    .is_none()
            );
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
                ..Settings::default()
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
            assert_eq!(messages.len(), 2);
            assert_eq!(
                object_field(&messages[0], "content"),
                Some(&Json::Text("log the system prompt".to_owned()))
            );
            assert_eq!(
                object_field(&messages[1], "content"),
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
            let callbacks = FailureCallbacks {
                registry: &CallbackRegistry::new(10),
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
            let state = state(None);
            let exception = ExceptionFacts {
                message: Some("server exploded".to_owned()),
                llm_provider: Some("openai".to_owned()),
                ..ExceptionFacts::default()
            };
            let builder = builder(Settings::default());
            let payload = builder
                .build(FailurePayloadInput {
                    state: &state,
                    exception: &exception,
                    start_time: start(),
                    end_time: end(),
                    error_str: "server exploded",
                })
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

impl std::io::Write for FailingSink {
    fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("sink is closed"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::other("sink is closed"))
    }
}

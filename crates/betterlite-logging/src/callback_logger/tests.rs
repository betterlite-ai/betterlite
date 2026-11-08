#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use tokio::sync::mpsc;

use super::CallbackLogger;
use super::error::LoggerError;
use super::event::{GuardrailRequestSnapshot, LoggerEvent};
use super::resolver::LoggerResolver;
use crate::fields::payload::StandardLoggingPayload;
use crate::registry::CallbackRegistry;
use crate::registry::Registration;
use crate::registry::error::RegistryError;
use crate::registry::identity::CallbackIdentity;
use crate::registry::phases::{CallbackList, CallbackPhase};
use crate::registry::sink::{CallbackEvent, CallbackSink};

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
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

#[derive(Clone, Default)]
struct Calls(Arc<Mutex<Vec<&'static str>>>);

impl Calls {
    fn record(&self, call: &'static str) {
        self.0.lock().unwrap().push(call);
    }

    fn names(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }
}

struct BaseLogger {
    name: String,
}

#[async_trait]
impl CallbackLogger for BaseLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }
}

struct QuietLogger {
    name: String,
}

#[async_trait]
impl CallbackLogger for QuietLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    fn turn_off_message_logging(&self) -> bool {
        true
    }

    fn redacts_messages_itself(&self) -> bool {
        true
    }

    fn message_logging(&self) -> bool {
        false
    }
}

struct SuccessOnlyLogger {
    name: String,
    calls: Calls,
}

#[async_trait]
impl CallbackLogger for SuccessOnlyLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    async fn async_log_success_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.calls.record("async_log_success_event");
        Ok(())
    }
}

struct SnapshotLogger {
    name: String,
    fingerprints: Arc<Mutex<Vec<Option<String>>>>,
}

#[async_trait]
impl CallbackLogger for SnapshotLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let fingerprint = event
            .guardrail_snapshot
            .as_ref()
            .map(|snapshot| snapshot.fingerprint.clone());
        self.fingerprints.lock().unwrap().push(fingerprint);
        Ok(())
    }
}

struct RecordingLogger {
    name: String,
    calls: Calls,
}

#[async_trait]
impl CallbackLogger for RecordingLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }

    fn log_success_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        self.calls.record("log_success_event");
        Ok(())
    }

    fn log_failure_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        self.calls.record("log_failure_event");
        Ok(())
    }

    fn log_stream_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        self.calls.record("log_stream_event");
        Ok(())
    }

    async fn async_log_success_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.calls.record("async_log_success_event");
        Ok(())
    }

    async fn async_log_failure_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.calls.record("async_log_failure_event");
        Ok(())
    }

    async fn async_log_stream_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.calls.record("async_log_stream_event");
        Ok(())
    }
}

struct FixedResolver {
    logger: Arc<dyn CallbackLogger>,
}

impl LoggerResolver for FixedResolver {
    fn resolve(
        &self,
        name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        if name == self.logger.callback_name() {
            return Ok(Some(Arc::clone(&self.logger)));
        }
        Ok(None)
    }
}

fn event(stream: bool) -> Arc<LoggerEvent> {
    let payload: StandardLoggingPayload = serde_json::from_str(PAYLOAD).unwrap();
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        stream,
        guardrail_snapshot: None,
    })
}

fn identity(name: &str) -> CallbackIdentity {
    CallbackIdentity {
        class_name: name.to_owned(),
        attributes: IndexMap::new(),
    }
}

fn logger_names(loggers: &[Arc<dyn CallbackLogger>]) -> Vec<String> {
    loggers
        .iter()
        .map(|logger| logger.callback_name().to_owned())
        .collect()
}

fn register(registry: &mut CallbackRegistry, name: &str, sink: CallbackSink, list: CallbackList) {
    assert_eq!(
        registry.register(identity(name), sink, list),
        Registration::Registered
    );
}

#[tokio::test]
async fn callback_logger_defaults_do_nothing_and_return_ok() {
    let logger = BaseLogger {
        name: "base".to_owned(),
    };
    assert_eq!(logger.callback_name(), "base");
    let event = event(false);
    logger.log_success_event(&event).unwrap();
    logger.log_failure_event(&event).unwrap();
    logger.log_stream_event(&event).unwrap();
    logger
        .async_log_success_event(Arc::clone(&event))
        .await
        .unwrap();
    logger
        .async_log_failure_event(Arc::clone(&event))
        .await
        .unwrap();
    logger
        .async_log_stream_event(Arc::clone(&event))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_logger_that_overrides_one_event_gets_only_that_call() {
    let calls = Calls::default();
    let logger = SuccessOnlyLogger {
        name: "custom".to_owned(),
        calls: calls.clone(),
    };
    let event = event(false);
    logger
        .async_log_success_event(Arc::clone(&event))
        .await
        .unwrap();
    assert_eq!(calls.names(), ["async_log_success_event"]);
    logger.log_success_event(&event).unwrap();
    logger.log_failure_event(&event).unwrap();
    logger.log_stream_event(&event).unwrap();
    logger
        .async_log_failure_event(Arc::clone(&event))
        .await
        .unwrap();
    logger
        .async_log_stream_event(Arc::clone(&event))
        .await
        .unwrap();
    assert_eq!(calls.names(), ["async_log_success_event"]);
}

#[test]
fn message_flags_default_to_false_and_a_logger_can_set_them() {
    let base = BaseLogger {
        name: "base".to_owned(),
    };
    assert!(!base.turn_off_message_logging());
    assert!(!base.redacts_messages_itself());
    let quiet = QuietLogger {
        name: "quiet".to_owned(),
    };
    assert!(quiet.turn_off_message_logging());
    assert!(quiet.redacts_messages_itself());
}

#[test]
fn message_logging_defaults_to_true_and_a_logger_can_turn_it_off() {
    let base = BaseLogger {
        name: "base".to_owned(),
    };
    assert!(base.message_logging());
    let quiet = QuietLogger {
        name: "quiet".to_owned(),
    };
    assert!(!quiet.message_logging());
}

#[test]
fn logger_event_carries_the_payload_dynamic_params_and_stream_flag() {
    let params = Json::Object(IndexMap::from([(
        "langfuse_public_key".to_owned(),
        Json::Text("pk-lf".to_owned()),
    )]));
    let payload: StandardLoggingPayload = serde_json::from_str(PAYLOAD).unwrap();
    let event = LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(params.clone()),
        stream: true,
        guardrail_snapshot: None,
    };
    assert_eq!(event.payload.model, "gpt-4o");
    assert_eq!(*event.dynamic_params, params);
    assert!(event.stream);
    assert!(event.guardrail_snapshot.is_none());
}

#[tokio::test]
async fn a_logger_reads_the_guardrail_snapshot_from_the_event() {
    let fingerprints = Arc::new(Mutex::new(Vec::new()));
    let logger = SnapshotLogger {
        name: "shadow".to_owned(),
        fingerprints: Arc::clone(&fingerprints),
    };
    let snapshot = GuardrailRequestSnapshot {
        body: IndexMap::from([(
            "messages".to_owned(),
            Json::List(vec![Json::Object(IndexMap::new())]),
        )]),
        fingerprint: "6f1c".to_owned(),
    };
    let payload: StandardLoggingPayload = serde_json::from_str(PAYLOAD).unwrap();
    let event = Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        stream: false,
        guardrail_snapshot: Some(Arc::new(snapshot)),
    });
    logger.async_log_success_event(event).await.unwrap();
    assert_eq!(
        fingerprints.lock().unwrap().clone(),
        [Some("6f1c".to_owned())]
    );
}

#[test]
fn logger_error_names_the_callback_that_failed() {
    let error = LoggerError::Delivery {
        callback: "langfuse".to_owned(),
        message: "the queue is full".to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "callback langfuse failed: the queue is full"
    );
}

#[rstest]
#[case::success(CallbackPhase::Success)]
#[case::failure(CallbackPhase::Failure)]
#[case::stream(CallbackPhase::Stream)]
fn loggers_lists_a_callbacks_logger_for_each_success_phase(#[case] phase: CallbackPhase) {
    let mut registry = CallbackRegistry::new(10);
    let logger: Arc<dyn CallbackLogger> = Arc::new(BaseLogger {
        name: "langfuse".to_owned(),
    });
    register(
        &mut registry,
        "langfuse",
        CallbackSink::Logger(Arc::clone(&logger)),
        CallbackList::Callbacks,
    );
    let listed = registry.loggers(phase);
    assert_eq!(listed.len(), 1);
    assert!(Arc::ptr_eq(&listed[0], &logger));
}

#[test]
fn loggers_lists_a_failure_only_logger_for_failure_alone() {
    let mut registry = CallbackRegistry::new(10);
    let logger: Arc<dyn CallbackLogger> = Arc::new(BaseLogger {
        name: "failure".to_owned(),
    });
    register(
        &mut registry,
        "failure",
        CallbackSink::Logger(logger),
        CallbackList::Failure,
    );
    assert_eq!(
        logger_names(&registry.loggers(CallbackPhase::Failure)),
        ["failure"]
    );
    assert!(registry.loggers(CallbackPhase::Success).is_empty());
    assert!(registry.loggers(CallbackPhase::Stream).is_empty());
}

#[test]
fn loggers_keeps_registration_order() {
    let mut registry = CallbackRegistry::new(10);
    let first: Arc<dyn CallbackLogger> = Arc::new(BaseLogger {
        name: "first".to_owned(),
    });
    let second: Arc<dyn CallbackLogger> = Arc::new(BaseLogger {
        name: "second".to_owned(),
    });
    register(
        &mut registry,
        "first",
        CallbackSink::Logger(first),
        CallbackList::Success,
    );
    register(
        &mut registry,
        "second",
        CallbackSink::Logger(second),
        CallbackList::Success,
    );
    assert_eq!(
        logger_names(&registry.loggers(CallbackPhase::Success)),
        ["first", "second"]
    );
}

#[test]
fn loggers_skips_tracing_and_channel_sinks() {
    let mut registry = CallbackRegistry::new(10);
    let logger: Arc<dyn CallbackLogger> = Arc::new(BaseLogger {
        name: "logger".to_owned(),
    });
    let (sender, _receiver) = mpsc::channel(4);
    register(
        &mut registry,
        "trace",
        CallbackSink::Tracing,
        CallbackList::Success,
    );
    register(
        &mut registry,
        "channel",
        CallbackSink::Channel(sender),
        CallbackList::Success,
    );
    register(
        &mut registry,
        "logger",
        CallbackSink::Logger(logger),
        CallbackList::Success,
    );
    assert_eq!(
        logger_names(&registry.loggers(CallbackPhase::Success)),
        ["logger"]
    );
}

#[test]
fn deliver_returns_ok_for_a_logger_sink_without_calling_it() {
    let calls = Calls::default();
    let logger = Arc::new(RecordingLogger {
        name: "recording".to_owned(),
        calls: calls.clone(),
    });
    let sink = CallbackSink::Logger(logger);
    sink.deliver(&CallbackEvent {
        phase: CallbackPhase::Success,
    })
    .unwrap();
    assert!(calls.names().is_empty());
}

#[test]
fn debug_of_a_logger_sink_shows_its_name() {
    let logger: Arc<dyn CallbackLogger> = Arc::new(BaseLogger {
        name: "langfuse".to_owned(),
    });
    let sink = CallbackSink::Logger(logger);
    assert_eq!(format!("{sink:?}"), "Logger(\"langfuse\")");
    assert_eq!(format!("{:?}", CallbackSink::Tracing), "Tracing");
}

#[test]
fn a_logger_resolver_returns_the_logger_or_none() {
    let logger: Arc<dyn CallbackLogger> = Arc::new(BaseLogger {
        name: "langfuse".to_owned(),
    });
    let resolver: Arc<dyn LoggerResolver> = Arc::new(FixedResolver { logger });
    let found = resolver
        .resolve("langfuse", &BTreeMap::new())
        .unwrap()
        .unwrap();
    assert_eq!(found.callback_name(), "langfuse");
    assert!(
        resolver
            .resolve("langsmith", &BTreeMap::new())
            .unwrap()
            .is_none()
    );
}

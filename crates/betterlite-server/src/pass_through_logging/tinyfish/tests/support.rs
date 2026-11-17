#![cfg(test)]

use std::env;
use std::io;
use std::process::Command;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::DateTime;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::records::TinyfishRun;

struct NoopDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoopDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

fn logging_init() -> LoggingInit {
    LoggingInit {
        model: "fixture-model".to_owned(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00")
            .expect("a valid timestamp"),
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

pub(super) fn logging_object() -> StreamLoggingObject {
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

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON literal")
}

pub(super) fn run(text: &str) -> TinyfishRun {
    serde_json::from_str(text).expect("a run object")
}

pub(super) fn decimal(text: &str) -> Decimal {
    text.parse().expect("a decimal literal")
}

// The child run sees the variable in its own environment, which the parent process cannot set for the resolver.
pub(super) fn rerun_test_with_environment(test: &str, variable: &str, value: &str, marker: &str) {
    let child = Command::new(env::current_exe().expect("the test binary path"))
        .args(["--exact", test, "--nocapture"])
        .env(marker, "1")
        .env(variable, value)
        .output()
        .expect("the test binary starts");
    let stdout = str::from_utf8(&child.stdout).expect("the child output is utf8");
    assert!(child.status.success(), "{child:?}");
    assert!(stdout.contains(marker), "{child:?}");
}

pub(super) fn kwargs(entries: &[(&str, Json)]) -> PassThroughLoggingKwargs {
    let mut collected = IndexMap::new();
    for (key, value) in entries {
        collected.insert((*key).to_owned(), value.clone());
    }
    PassThroughLoggingKwargs { entries: collected }
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

pub(super) fn detail_text(logging_obj: &StreamLoggingObject, name: &str) -> Option<String> {
    match details(logging_obj).get(name) {
        Some(Json::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

pub(super) fn detail_cost(logging_obj: &StreamLoggingObject) -> Option<String> {
    match details(logging_obj).get("response_cost") {
        Some(Json::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for Captured {
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

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

// The fallback paths only warn, so a captured log line is the only proof they ran.
pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let buffer = Captured::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

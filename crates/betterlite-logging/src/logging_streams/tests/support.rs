use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use thiserror::Error;

use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::ResponseStamp;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::usage::Usage;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::types::{CallbackPolicy, DynamicCallbacks, RequestLoggingFlags};
use crate::logging_state::{LoggingInit, LoggingState};
use crate::logging_streams::assembly::{PromptCounter, StreamAssembly, StreamAssemblyError};
use crate::logging_streams::cache_assembly::StreamChunks;
use crate::logging_streams::flush::StreamCollector;
use crate::logging_success::handler::SuccessHandler;
use crate::logging_success::ports::{StandardPayloadBuilder, StreamAssembler, SuccessClock};
use crate::logging_success::types::{SuccessMode, SuccessResult};
use crate::registry::CallbackRegistry;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

pub(super) fn state(stream: Option<bool>) -> LoggingState {
    state_with(stream, "completion", "gpt-4o")
}

pub(super) fn state_with(stream: Option<bool>, call_type: &str, model: &str) -> LoggingState {
    let init = LoggingInit {
        model: model.to_owned(),
        call_type: call_type.to_owned(),
        start_time: time("2024-01-01T00:00:00+00:00"),
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
    };
    LoggingState::new(
        init,
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

#[derive(Default)]
pub(super) struct RecordingBuilder {
    responses: Mutex<Vec<Json>>,
}

impl RecordingBuilder {
    pub(super) fn responses(&self) -> Vec<Json> {
        self.responses.lock().unwrap().clone()
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
        self.responses.lock().unwrap().push(response.clone());
        None
    }
}

pub(super) struct SwitchedAssembler {
    pub(super) complete: Option<Json>,
}

impl StreamAssembler for SwitchedAssembler {
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
            SuccessResult::Chunk(_) => self.complete.clone(),
            SuccessResult::Chat(_) | SuccessResult::Embeddings(_) | SuccessResult::Other(_) => {
                Some(result.response().clone())
            }
        }
    }
}

pub(super) struct FixedClock {
    pub(super) now: DateTime<FixedOffset>,
}

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

pub(super) struct HandlerFixture {
    pub(super) registry: CallbackRegistry,
    pub(super) builder: RecordingBuilder,
    pub(super) assembler: SwitchedAssembler,
    pub(super) clock: FixedClock,
    counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    excluded_fields: Vec<String>,
    known: Box<dyn Fn(&str) -> bool + Send + Sync>,
    stamp: ResponseStamp,
}

impl HandlerFixture {
    pub(super) fn new(complete: Option<Json>) -> Self {
        Self {
            registry: CallbackRegistry::new(10),
            builder: RecordingBuilder::default(),
            assembler: SwitchedAssembler { complete },
            clock: FixedClock {
                now: time("2024-01-01T00:00:02+00:00"),
            },
            counters: Vec::new(),
            excluded_fields: Vec::new(),
            known: Box::new(|_name: &str| false),
            stamp: ResponseStamp {
                id: "chatcmpl-placeholder".to_owned(),
                created: 1_704_067_200,
            },
        }
    }

    pub(super) fn handler(&self) -> SuccessHandler<'_> {
        SuccessHandler {
            registry: &self.registry,
            payload_builder: &self.builder,
            stream_assembler: &self.assembler,
            clock: &self.clock,
            failure_counters: &self.counters,
            excluded_fields: &self.excluded_fields,
            known_callback: self.known.as_ref(),
            stamp: &self.stamp,
            privacy: Settings::default(),
        }
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub(super) struct CollectorFailure {
    pub(super) message: &'static str,
}

pub(super) struct FakeCollector {
    pub(super) result: Option<SuccessResult>,
    pub(super) failure: Option<&'static str>,
    pub(super) seen_model: Mutex<Option<String>>,
}

impl FakeCollector {
    pub(super) fn returning(result: SuccessResult) -> Self {
        Self {
            result: Some(result),
            failure: None,
            seen_model: Mutex::new(None),
        }
    }

    pub(super) fn empty() -> Self {
        Self {
            result: None,
            failure: None,
            seen_model: Mutex::new(None),
        }
    }

    pub(super) fn failing(message: &'static str) -> Self {
        Self {
            result: None,
            failure: Some(message),
            seen_model: Mutex::new(None),
        }
    }

    pub(super) fn seen_model(&self) -> Option<String> {
        self.seen_model.lock().unwrap().clone()
    }
}

impl StreamCollector for FakeCollector {
    type Error = CollectorFailure;

    fn build_logged_response(
        &self,
        state: &mut LoggingState,
    ) -> Result<Option<SuccessResult>, Self::Error> {
        *self.seen_model.lock().unwrap() = Some(state.model.clone());
        if let Some(message) = self.failure {
            return Err(CollectorFailure { message });
        }
        Ok(self.result.clone())
    }
}

#[derive(Debug, Error)]
#[error("the builder failed")]
pub(super) struct BuilderFailure;

pub(super) struct FakeAssembly {
    pub(super) response: Option<Json>,
    pub(super) failure: bool,
    pub(super) usage: Usage,
    pub(super) seen_counter: Mutex<Option<Option<i64>>>,
}

impl FakeAssembly {
    pub(super) fn complete(response: Json) -> Self {
        Self {
            response: Some(response),
            failure: false,
            usage: Usage::default(),
            seen_counter: Mutex::new(None),
        }
    }

    pub(super) fn empty() -> Self {
        Self {
            response: None,
            failure: false,
            usage: Usage::default(),
            seen_counter: Mutex::new(None),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            response: None,
            failure: true,
            usage: Usage::default(),
            seen_counter: Mutex::new(None),
        }
    }

    pub(super) fn counter_value(&self) -> Option<Option<i64>> {
        *self.seen_counter.lock().unwrap()
    }
}

impl StreamAssembly for FakeAssembly {
    fn assemble(
        &self,
        count_prompt_tokens: Option<&PromptCounter<'_>>,
    ) -> Result<Option<Json>, StreamAssemblyError> {
        *self.seen_counter.lock().unwrap() =
            Some(count_prompt_tokens.and_then(|counter| counter().ok()));
        if self.failure {
            return Err(StreamAssemblyError::Builder(Box::new(BuilderFailure)));
        }
        Ok(self.response.clone())
    }

    fn total_usage(&self) -> Usage {
        self.usage.clone()
    }
}

pub(super) struct FakeChunks {
    pub(super) pushes: Mutex<Vec<Json>>,
    pub(super) built: Option<Json>,
    pub(super) failure: bool,
    build_calls: Mutex<usize>,
}

impl FakeChunks {
    pub(super) fn new(built: Option<Json>) -> Self {
        Self {
            pushes: Mutex::new(Vec::new()),
            built,
            failure: false,
            build_calls: Mutex::new(0),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            pushes: Mutex::new(Vec::new()),
            built: None,
            failure: true,
            build_calls: Mutex::new(0),
        }
    }

    pub(super) fn pushes(&self) -> Vec<Json> {
        self.pushes.lock().unwrap().clone()
    }

    pub(super) fn build_calls(&self) -> usize {
        *self.build_calls.lock().unwrap()
    }
}

impl StreamChunks<Json> for FakeChunks {
    fn push(&mut self, chunk: &Json) {
        self.pushes.lock().unwrap().push(chunk.clone());
    }

    fn build(&self) -> Result<Option<Json>, StreamAssemblyError> {
        *self.build_calls.lock().unwrap() += 1;
        if self.failure {
            return Err(StreamAssemblyError::Builder(Box::new(BuilderFailure)));
        }
        Ok(self.built.clone())
    }
}

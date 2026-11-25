use std::io;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::cost::recording::PortFailure;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::passthrough_handler::types::StreamedResponse;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::super::{
    PassthroughChatLogger, PassthroughLoggingError, PassthroughPayload, PassthroughPayloadInput,
    PassthroughProvider, PassthroughStage, UpstreamChatReply,
};
use crate::pass_through_logging::response_costing::{ResponseCostRequest, ResponseCostService};
use crate::pass_through_logging::response_payload::ResponseLoggingKwargs;

pub(crate) const START: &str = "2024-01-01T00:00:00+00:00";
pub(crate) const END: &str = "2024-01-01T00:00:01+00:00";

pub(crate) fn timestamp(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).expect("a valid timestamp")
}

pub(crate) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("a JSON object")
}

pub(crate) fn json_object(text: &str) -> Json {
    Json::Object(object(text))
}

pub(crate) fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    let Json::Object(fields) = value else {
        panic!("the value is an object");
    };
    fields.get(name).expect("the object carries the key")
}

pub(crate) fn logging_state() -> LoggingState {
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let init = LoggingInit {
        model: String::new(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: timestamp(START),
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
    };
    LoggingState::new(
        init,
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state")
}

pub(crate) fn logger(
    provider: &Arc<FakeProvider>,
    payload: &Arc<FakePayload>,
) -> PassthroughChatLogger {
    PassthroughChatLogger::new(
        Arc::<FakeProvider>::clone(provider),
        Arc::<FakePayload>::clone(payload),
    )
}

// The provider as the handlers see it: the models it was asked about and the answers it gives.
pub(crate) struct FakeProvider {
    models: Mutex<Vec<String>>,
    chat: Mutex<Option<Json>>,
    streamed: Mutex<Option<StreamedResponse>>,
    stream_fails: bool,
}

impl FakeProvider {
    pub(crate) fn chat(response: Json) -> Self {
        Self {
            models: Mutex::new(Vec::new()),
            chat: Mutex::new(Some(response)),
            streamed: Mutex::new(None),
            stream_fails: false,
        }
    }

    // The streaming shape with the failure flag set: the transform and the stream build both fail.
    pub(crate) fn failing() -> Self {
        let mut provider = Self::streaming(None);
        provider.stream_fails = true;
        provider
    }

    pub(crate) fn streaming(streamed: Option<StreamedResponse>) -> Self {
        Self {
            models: Mutex::new(Vec::new()),
            chat: Mutex::new(None),
            streamed: Mutex::new(streamed),
            stream_fails: false,
        }
    }

    pub(crate) fn seen(&self) -> Vec<String> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PassthroughProvider for FakeProvider {
    fn provider_name(&self) -> ProviderId {
        ProviderId::Cohere
    }

    fn transform_chat_response(
        &self,
        model: &str,
        _upstream: &UpstreamChatReply<'_>,
    ) -> Result<Json, PassthroughLoggingError> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        let response = self
            .chat
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        response.map_or_else(
            || {
                Err(PassthroughStage::Transform
                    .failed(io::Error::other("the fake transform failed")))
            },
            Ok,
        )
    }

    fn build_complete_streaming_response(
        &self,
        _all_chunks: &[String],
        model: &str,
    ) -> Result<Option<StreamedResponse>, PassthroughLoggingError> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        if self.stream_fails {
            return Err(PassthroughStage::StreamBuild
                .failed(io::Error::other("the fake stream build failed")));
        }
        let streamed = self
            .streamed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        Ok(streamed)
    }
}

// The payload collaborator as the handlers see it: the models it priced and the kwargs it answers with.
pub(crate) struct FakePayload {
    models: Mutex<Vec<String>>,
    call_ids: Mutex<Vec<String>>,
    inputs: Mutex<Vec<ResponseLoggingKwargs>>,
    end_user_bodies: Mutex<Vec<Option<JsonObject>>>,
    kwargs: ResponseLoggingKwargs,
    end_user: Option<String>,
}

impl FakePayload {
    pub(crate) fn new(kwargs: ResponseLoggingKwargs) -> Self {
        Self {
            models: Mutex::new(Vec::new()),
            call_ids: Mutex::new(Vec::new()),
            inputs: Mutex::new(Vec::new()),
            end_user_bodies: Mutex::new(Vec::new()),
            kwargs,
            end_user: None,
        }
    }

    pub(crate) fn with_end_user(mut self, user: &str) -> Self {
        self.end_user = Some(user.to_owned());
        self
    }

    pub(crate) fn seen(&self) -> Vec<String> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn call_ids(&self) -> Vec<String> {
        self.call_ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn inputs(&self) -> Vec<ResponseLoggingKwargs> {
        self.inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn end_user_bodies(&self) -> Vec<Option<JsonObject>> {
        self.end_user_bodies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PassthroughPayload for FakePayload {
    fn build(&self, input: PassthroughPayloadInput<'_>) -> ResponseLoggingKwargs {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(input.model.to_owned());
        self.inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(input.kwargs.clone());
        self.call_ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(input.call_log.litellm_call_id.clone());
        if let Json::Object(fields) = input.response {
            fields.insert(
                "payload_model".to_owned(),
                Json::Text(input.model.to_owned()),
            );
        }
        let mut kwargs = self.kwargs.clone();
        kwargs.model = Some(input.model.to_owned());
        kwargs
    }

    fn end_user(&self, request_body: Option<&JsonObject>) -> Option<String> {
        self.end_user_bodies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request_body.cloned());
        self.end_user.clone()
    }
}

// B3630's cost port at a fixed price, recording the models it priced.
pub(crate) struct FixedCost {
    price: Decimal,
    models: Mutex<Vec<String>>,
}

impl FixedCost {
    pub(crate) fn new(price: Decimal) -> Self {
        Self {
            price,
            models: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn models(&self) -> Vec<String> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResponseCostService for FixedCost {
    fn completion_cost(&self, request: &ResponseCostRequest<'_>) -> Result<Decimal, PortFailure> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.model.to_owned());
        Ok(self.price)
    }
}

// The standard payload builder that records each response it is handed and builds nothing.
#[derive(Default)]
pub(crate) struct RecordingBuilder {
    responses: Mutex<Vec<Json>>,
}

impl RecordingBuilder {
    pub(crate) fn responses(&self) -> Vec<Json> {
        self.responses
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
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(response.clone());
        None
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
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

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_logs(level: Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

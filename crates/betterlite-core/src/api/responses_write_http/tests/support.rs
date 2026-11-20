use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::response_events::RawResponse;
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_openai::response_resources::request::{
    ResourceOperation, ResourceRequest, resource_request,
};
use betterlite_openai::response_resources::response::{TypedReply, decode_record, response_reply};
use betterlite_prompts::arguments::json::Object as ResourceObject;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::HeaderMap as HttpHeaders;
use indexmap::IndexMap;
use secrecy::SecretString;
use tracing::Event;
use tracing::field::Field;
use tracing::field::Visit;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::Subscriber;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::Context;
use tracing_subscriber::layer::Layer;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use crate::api::responses_write_http::config::{
    OpenAiResponsesConfig, ResponsesConfig, WriteParams,
};
use crate::api::responses_write_http::error::WriteError;
use crate::api::responses_write_http::service::{
    ClientKind, ClientSource, DeleteCall, LogContext, PreparedWrite, SharedSession, SuppliedClient,
    WriteCall, WriteDeps, WriteRequest, WriteTarget, WriteTransport, prepare,
};
use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, HeaderMap, UrlFailure,
};

#[derive(Clone, Default)]
pub(crate) struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(crate) fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(crate) fn new(call_type: &str) -> Self {
        let init: LoggingInit = serde_json::from_str(&format!(
            r#"{{"model":"responses","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        ))
        .expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            redactor,
            sink: Sink,
            state,
            now,
        }
    }

    pub(crate) fn prepare<'deps, C, S>(
        &mut self,
        deps: &'deps WriteDeps<'deps, C, S>,
        mode: ClientKind,
        call: &WriteCall<'_>,
        events: &Events,
    ) -> Result<PreparedWrite<'deps, C, S>, WriteError>
    where
        C: ResponsesConfig,
        S: ClientSource,
    {
        let logger = |_details: &Json| -> Result<(), PrePostError> {
            events.push("log");
            Ok(())
        };
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: Some(&logger),
            now: self.now,
        };
        prepare(
            deps,
            mode,
            call,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
    }

    pub(crate) fn logged(&self, name: &str) -> Option<&Json> {
        let Json::Object(details) = self.state.model_call_details.get("additional_args")? else {
            return None;
        };
        details.get(name)
    }

    pub(crate) fn input(&self) -> Option<&Json> {
        self.state.model_call_details.get("input")
    }

    pub(crate) fn details(&self, name: &str) -> Option<&Json> {
        self.state.model_call_details.get(name)
    }
}

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: HttpHeaders,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.insert(
            name,
            http::HeaderValue::from_str(value).expect("the header value"),
        );
        self
    }
}

#[derive(Clone)]
pub(crate) struct Wire {
    reply: Reply,
    failure: Arc<Mutex<Option<TransportError>>>,
    requests: Arc<Mutex<Vec<WriteRequest>>>,
    events: Events,
}

impl Wire {
    pub(crate) fn replying(reply: Reply, events: &Events) -> Self {
        Self {
            reply,
            failure: Arc::default(),
            requests: Arc::default(),
            events: events.clone(),
        }
    }

    pub(crate) fn failing(error: TransportError, events: &Events) -> Self {
        let wire = Self::replying(Reply::json(200, "{}"), events);
        *wire.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        wire
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<WriteRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl WriteTransport for Wire {
    fn send(
        &self,
        request: WriteRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.events.push("send");
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if let Some(error) = failure {
            return ready(Err(error));
        }
        ready(Ok(UpstreamResponse {
            status: self.reply.status,
            headers: self.reply.headers.clone(),
            body: self.reply.body.clone(),
        }))
    }
}

pub(crate) struct Source {
    wire: Wire,
    calls: Arc<Mutex<Vec<(Option<bool>, bool)>>>,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self {
            wire,
            calls: Arc::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<(Option<bool>, bool)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(
        &self,
        ssl_verify: Option<bool>,
        shared_session: Option<SharedSession>,
    ) -> Result<Wire, ClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((ssl_verify, shared_session.is_some()));
        Ok(self.wire.clone())
    }
}

// The error converter's answers are observable through this config
pub(crate) struct RecordingConfig {
    pub(crate) base: String,
    payload: Mutex<Option<ResourceObject>>,
    errors: Mutex<Vec<(String, u16, HeaderMap)>>,
    media_type: bool,
}

impl RecordingConfig {
    pub(crate) fn new(base: &str) -> Self {
        Self {
            base: base.to_owned(),
            payload: Mutex::default(),
            errors: Mutex::default(),
            media_type: true,
        }
    }

    // A provider whose validate_environment leaves the media type unset, as hosted_vllm's responses config does
    pub(crate) fn without_media_type(base: &str) -> Self {
        Self {
            media_type: false,
            ..Self::new(base)
        }
    }

    pub(crate) fn set_payload(&self, json: &str) {
        let payload = serde_json::from_str(json).expect("the payload fixture is valid");
        *self.payload.lock().unwrap_or_else(PoisonError::into_inner) = Some(payload);
    }

    pub(crate) fn errors(&self) -> Vec<(String, u16, HeaderMap)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn resource(
        &self,
        response_id: &str,
        api_base: &str,
        operation: ResourceOperation,
    ) -> Result<ResourceRequest, ResponseResourceError> {
        let mut request = resource_request(response_id, api_base, operation)?;
        if let Some(payload) = self
            .payload
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            request.payload = payload;
        }
        Ok(request)
    }
}

impl ResponsesConfig for RecordingConfig {
    fn validate_environment(
        &self,
        mut headers: BTreeMap<String, String>,
        _params: &WriteParams,
    ) -> BTreeMap<String, String> {
        if self.media_type {
            headers
                .entry(String::from("Content-Type"))
                .or_insert_with(|| String::from("application/json"));
        }
        headers.insert(
            String::from("Authorization"),
            String::from("Bearer sk-provider"),
        );
        headers
    }

    fn complete_url(&self, _params: &WriteParams) -> Result<String, UrlFailure> {
        Ok(self.base.clone())
    }

    fn delete_request(
        &self,
        response_id: &str,
        api_base: &str,
    ) -> Result<ResourceRequest, ResponseResourceError> {
        self.resource(response_id, api_base, ResourceOperation::Delete)
    }

    fn cancel_request(
        &self,
        response_id: &str,
        api_base: &str,
    ) -> Result<ResourceRequest, ResponseResourceError> {
        self.resource(response_id, api_base, ResourceOperation::Cancel)
    }

    fn delete_response(
        &self,
        reply: &RawResponse<'_>,
    ) -> Result<DeleteResponseResult, ResponseResourceError> {
        decode_record(reply)
    }

    fn cancel_response(
        &self,
        reply: &RawResponse<'_>,
    ) -> Result<TypedReply, ResponseResourceError> {
        response_reply(reply)
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((message.to_owned(), status_code, headers.clone()));
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

pub(crate) fn openai_settings() -> OpenAiCredentialsSettings {
    OpenAiCredentialsSettings::default()
}

pub(crate) fn openai_config<'settings>(
    settings: &'settings OpenAiCredentialsSettings,
    env_api_key: &str,
) -> OpenAiResponsesConfig<'settings> {
    OpenAiResponsesConfig::new(
        settings,
        LiteLlmGlobals::default(),
        Some(SecretString::from(env_api_key)),
        None,
    )
}

pub(crate) fn params(api_base: &str) -> WriteParams {
    WriteParams {
        api_base: Some(api_base.to_owned()),
        api_key: None,
        ssl_verify: None,
    }
}

pub(crate) fn extra_body(json: &str) -> IndexMap<String, Json> {
    serde_json::from_str(json).expect("the extra body fixture is valid")
}

pub(crate) fn caller_headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) const fn delete_call<'call>(
    response_id: &'call str,
    params: &'call WriteParams,
    extra_headers: Option<&'call BTreeMap<String, String>>,
    extra_body: Option<&'call IndexMap<String, Json>>,
    timeout: Option<Duration>,
) -> WriteCall<'call> {
    WriteCall::Delete(DeleteCall {
        target: WriteTarget {
            response_id,
            params,
            extra_headers,
            timeout,
        },
        extra_body,
    })
}

pub(crate) fn body_bytes(request: &WriteRequest) -> Option<&[u8]> {
    match &request.body {
        RequestBody::Json(body) => Some(body),
        RequestBody::Bytes { data, .. } => Some(data),
        RequestBody::Empty | RequestBody::Multipart { .. } => None,
    }
}

pub(crate) fn body_json(request: &WriteRequest) -> Json {
    let body = body_bytes(request).expect("the request carries a JSON body");
    serde_json::from_slice(body).expect("the request body is JSON")
}

pub(crate) const fn write_deps<'deps, C, S>(
    config: &'deps C,
    source: &'deps S,
    supplied: Option<SuppliedClient<'deps, S::Client>>,
    shared_session: Option<SharedSession>,
) -> WriteDeps<'deps, C, S>
where
    C: ResponsesConfig,
    S: ClientSource,
{
    WriteDeps {
        config,
        source,
        supplied,
        shared_session,
    }
}

#[derive(Clone, Default)]
pub(crate) struct Capture(pub(crate) Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        if !visitor.0.is_empty() {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(visitor.0);
        }
    }
}

pub(crate) fn capturing() -> (Capture, DefaultGuard) {
    let captured = Capture::default();
    let guard = set_default(registry().with(captured.clone()));
    (captured, guard)
}

pub(crate) fn captured_messages(captured: &Capture) -> Vec<String> {
    captured
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

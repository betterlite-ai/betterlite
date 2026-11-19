use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse, base_builder};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, once};
use http::HeaderMap as HttpHeaders;
use indexmap::IndexMap;

use crate::api::genai_http::config::{AuthCall, AuthTarget, GenerateContentHttpConfig};
use crate::api::genai_http::error::GenAiHttpError;
use crate::api::genai_http::service::{
    AsyncGenerateContentOutcome, ClientSource, GenAiRequest, GenAiTransport, GenerateContentCall,
    GenerateContentDeps, GenerateContentOutcome, StreamReply, WireReply, async_generate_content,
    generate_content,
};
use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, HeaderMap, header_map,
};
use crate::api::vector_store_http_write::service::LogContext;
use crate::genai_setup::config::{GenerateContentProviderConfig, GenerateContentRequest};
use crate::genai_setup::params::GenerateContentLiteLLMParams;

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
    pub(crate) state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(crate) fn new(call_type: &str) -> Self {
        let init: LoggingInit = serde_json::from_str(&format!(
            r#"{{"model":"gemini-2.0-flash","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
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

    pub(crate) async fn sync_call<'deps, C, S>(
        &mut self,
        deps: &GenerateContentDeps<'deps, C, S>,
        call: &GenerateContentCall<'_>,
        events: &Events,
    ) -> Result<GenerateContentOutcome<C::Response>, GenAiHttpError<C::Error>>
    where
        C: GenerateContentHttpConfig,
        S: ClientSource,
    {
        let logger = |_details: &DecisionJson| -> Result<(), PrePostError> {
            events.push("log");
            Ok(())
        };
        let env = pre_post_env(&self.redactor, &self.sink, self.now, &logger);
        generate_content(
            deps,
            call,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
        .await
    }

    pub(crate) async fn async_call<'deps, C, S>(
        &mut self,
        deps: &GenerateContentDeps<'deps, C, S>,
        call: &GenerateContentCall<'_>,
        events: &Events,
    ) -> Result<AsyncGenerateContentOutcome<C::Response>, GenAiHttpError<C::Error>>
    where
        C: GenerateContentHttpConfig,
        S: ClientSource,
    {
        let logger = |_details: &DecisionJson| -> Result<(), PrePostError> {
            events.push("log");
            Ok(())
        };
        let env = pre_post_env(&self.redactor, &self.sink, self.now, &logger);
        async_generate_content(
            deps,
            call,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
        .await
    }

    pub(crate) fn logged(&self, name: &str) -> Option<&DecisionJson> {
        let DecisionJson::Object(details) = self.state.model_call_details.get("additional_args")?
        else {
            return None;
        };
        details.get(name)
    }

    pub(crate) fn input(&self) -> Option<&DecisionJson> {
        self.state.model_call_details.get("input")
    }
}

// Built from the fixture's fields rather than through a method, so the call can still borrow the state mutably
fn pre_post_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Sink,
    now: DateTime<FixedOffset>,
    logger: &'env dyn Fn(&DecisionJson) -> Result<(), PrePostError>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: Some(logger),
        now,
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

pub(crate) struct Recorded {
    pub(crate) request: GenAiRequest,
    pub(crate) streaming: bool,
}

#[derive(Clone)]
pub(crate) struct Wire {
    name: &'static str,
    reply: Reply,
    stream_body: Option<Vec<u8>>,
    streaming_always: bool,
    failure: Arc<Mutex<Option<TransportError>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
    events: Events,
    default_timeout: Duration,
}

impl Wire {
    pub(crate) fn replying(name: &'static str, reply: Reply, events: &Events) -> Self {
        Self {
            name,
            reply,
            stream_body: None,
            streaming_always: false,
            failure: Arc::default(),
            requests: Arc::default(),
            events: events.clone(),
            default_timeout: Duration::from_secs(60),
        }
    }

    pub(crate) fn streaming(name: &'static str, reply: Reply, events: &Events) -> Self {
        let stream_body = Some(reply.body.clone());
        Self {
            stream_body,
            ..Self::replying(name, reply, events)
        }
    }

    pub(crate) fn failing(name: &'static str, error: TransportError, events: &Events) -> Self {
        let wire = Self::replying(name, Reply::json(200, "{}"), events);
        *wire.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        wire
    }

    pub(crate) fn always_streaming(mut self) -> Self {
        self.streaming_always = true;
        self
    }

    pub(crate) fn with_default_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<Recorded>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl GenAiTransport for Wire {
    fn default_timeout(&self) -> Duration {
        self.default_timeout
    }

    fn send(
        &self,
        request: GenAiRequest,
        streaming: bool,
    ) -> impl Future<Output = Result<WireReply, TransportError>> + Send {
        self.events.push(self.name);
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded { request, streaming });
        if let Some(error) = failure {
            return ready(Err(error));
        }
        if let Some(body) = &self.stream_body
            && (streaming || self.streaming_always)
        {
            let answer = StreamReply {
                headers: self.reply.headers.clone(),
                body: body_stream(body.clone()),
            };
            return ready(Ok(WireReply::Streaming(answer)));
        }
        ready(Ok(WireReply::Buffered(UpstreamResponse {
            status: self.reply.status,
            headers: self.reply.headers.clone(),
            body: self.reply.body.clone(),
        })))
    }
}

fn body_stream(body: Vec<u8>) -> BoxStream<'static, Result<Vec<u8>, HttpError>> {
    once(ready(Ok(body))).boxed()
}

pub(crate) struct Source {
    wire: Wire,
    calls: Arc<Mutex<Vec<Option<Json>>>>,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self {
            wire,
            calls: Arc::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Option<Json>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(&self, ssl_verify: Option<&Json>) -> Result<Wire, ClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify.cloned());
        Ok(self.wire.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthRecord {
    pub(crate) api_base: Option<String>,
    pub(crate) model: String,
    pub(crate) stream: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("the test provider config cannot build the call")]
pub(crate) struct TestConfigError;

// The provider config the handler talks to: it records the auth call, builds a Gemini-shaped body, and answers a
// transform or decode failure when a case asks for one
pub(crate) struct TestConfig {
    pub(crate) base: String,
    auth_failure: bool,
    transform_failure: bool,
    decode_failure: bool,
    text_body: bool,
    extra_headers: Vec<(String, String)>,
    auth_calls: Mutex<Vec<AuthRecord>>,
    errors: Mutex<Vec<(String, u16, HeaderMap)>>,
}

impl TestConfig {
    pub(crate) fn new(base: &str) -> Self {
        Self {
            base: base.to_owned(),
            auth_failure: false,
            transform_failure: false,
            decode_failure: false,
            text_body: false,
            extra_headers: Vec::new(),
            auth_calls: Mutex::default(),
            errors: Mutex::default(),
        }
    }

    pub(crate) fn provider_header(mut self, name: &str, value: &str) -> Self {
        self.extra_headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub(crate) fn failing_auth(mut self) -> Self {
        self.auth_failure = true;
        self
    }

    pub(crate) fn failing_transform(mut self) -> Self {
        self.transform_failure = true;
        self
    }

    pub(crate) fn failing_decode(mut self) -> Self {
        self.decode_failure = true;
        self
    }

    pub(crate) fn with_text_body(mut self) -> Self {
        self.text_body = true;
        self
    }

    pub(crate) fn auth_calls(&self) -> Vec<AuthRecord> {
        self.auth_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn errors(&self) -> Vec<(String, u16, HeaderMap)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl GenerateContentProviderConfig for TestConfig {
    type Error = TestConfigError;

    fn map_optional_params(&self, config: &Object, _model: &str) -> Object {
        config.clone()
    }

    fn transform_request(
        &self,
        request: &GenerateContentRequest<'_>,
    ) -> Result<Json, TestConfigError> {
        if self.transform_failure {
            return Err(TestConfigError);
        }
        if self.text_body {
            return Ok(Json::Text("not an object".to_owned()));
        }
        let mut body = Object::new();
        body.insert("model".to_owned(), Json::Text(request.model.to_owned()));
        body.insert("contents".to_owned(), request.contents.clone());
        body.insert(
            "generationConfig".to_owned(),
            Json::Object(request.generation_config.clone()),
        );
        if let Some(tools) = request.tools {
            body.insert("tools".to_owned(), tools.clone());
        }
        if let Some(system) = request.system_instruction {
            body.insert("systemInstruction".to_owned(), system.clone());
        }
        Ok(Json::Object(body))
    }

    fn request_top_level_fields(&self) -> &'static [&'static str] {
        &[]
    }
}

impl GenerateContentHttpConfig for TestConfig {
    type Response = Json;

    fn auth_token_and_url(
        &self,
        call: &AuthCall<'_>,
    ) -> impl Future<Output = Result<AuthTarget, TestConfigError>> + Send {
        self.auth_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(AuthRecord {
                api_base: call.api_base.map(ToOwned::to_owned),
                model: call.model.to_owned(),
                stream: call.stream,
            });
        if self.auth_failure {
            return ready(Err(TestConfigError));
        }
        let operation = if call.stream {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        let mut headers = IndexMap::new();
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
        headers.insert("x-goog-api-key".to_owned(), "provider-key".to_owned());
        for (name, value) in &self.extra_headers {
            headers.insert(name.clone(), value.clone());
        }
        ready(Ok(AuthTarget {
            url: format!("{}/v1beta/models/{}:{operation}", self.base, call.model),
            headers,
        }))
    }

    fn decode_response(&self, response: &UpstreamResponse) -> Result<Json, BaseLlmException> {
        if self.decode_failure {
            return Err(self.error_class(
                "Error transforming generate content response: the body is not JSON",
                response.status,
                header_map(&response.headers),
            ));
        }
        serde_json::from_slice(&response.body).map_err(|source| {
            self.error_class(
                &format!("Error transforming generate content response: {source}"),
                response.status,
                header_map(&response.headers),
            )
        })
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

pub(crate) fn kwargs(json: &str) -> GenerateContentLiteLLMParams {
    let object: Object = serde_json::from_str(json).expect("the kwargs fixture is valid");
    GenerateContentLiteLLMParams::from_kwargs(&object).expect("the kwargs fixture is valid")
}

// The real pooled client behind the same seam, so a case can time out against a fixture that answers late
pub(crate) struct ProxiedSource {
    client: ProxiedClient,
}

impl ProxiedSource {
    pub(crate) fn new(limits: TransportLimits) -> Self {
        let policy = Arc::new(
            ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
                .expect("the proxy policy resolves"),
        );
        let client = ProxiedClient::build(base_builder().http1_only(), &policy, limits)
            .expect("the proxied client builds");
        Self { client }
    }
}

impl ClientSource for ProxiedSource {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<&Json>) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}

pub(crate) const IMPATIENT: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(100),
    ..TransportLimits::DEFAULT
};

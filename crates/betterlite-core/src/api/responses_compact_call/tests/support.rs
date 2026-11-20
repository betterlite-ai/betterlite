use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_context::non_default_params::Params;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use betterlite_openai::response_events::{
    ApiKeySources, RawResponse, ReplyBody, ResponsesProvider, ResponsesReply, WorkloadIdentity,
    get_complete_url, transform_response, validate_environment,
};
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use betterlite_openai::response_resources::request::{
    CompactRequest as ProviderCompactRequest, ResourceRequest, compact_request,
};
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::responses_compact_call::call::CompactResponsesCall;
use crate::api::responses_compact_call::deps::{CompactConfigFactory, ConfigRequest};
use crate::api::responses_compact_call::error::CompactParamError;
use crate::api::responses_compact_http::config::{
    CompactConfig, CompactParams, CompactTransformRequest, Headers, SignRequest, Signed,
};
use crate::api::responses_compact_http::error::ConfigFailure;
use crate::api::responses_compact_http::service::{
    ClientSource, CompactRequest, CompactTransport, SharedSession,
};
use crate::api::responses_parameters::provider::ResponsesParamProvider;
use crate::api::responses_provider_selection::provider_config::{ConfigLookup, NativeConfig};
use crate::api::responses_provider_selection::resolution::ModelResolver;
use crate::api::vector_store_http_write::error::{BaseLlmException, ClientError, HeaderMap};

pub(crate) const COMPACT_OK: &str = r#"{"id":"resp_compact","created_at":100,"object":"response","status":"completed","model":"gpt-4o","output":[]}"#;
pub(crate) const ENCODED_RESPONSE: &str = "resp_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOm9wZW5haTttb2RlbF9pZDpkZXBsb3ltZW50X2E7cmVzcG9uc2VfaWQ6cmVzcF9jb21wYWN0";

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

pub(crate) struct Harness {
    pub(crate) redactor: Redactor,
    pub(crate) sink: Sink,
    pub(crate) now: DateTime<FixedOffset>,
    pub(crate) state: LoggingState,
}

impl Harness {
    pub(crate) fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"gpt-4o","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
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
            now,
            state,
        }
    }
}

pub(crate) fn base_env<'env>(
    redactor: &'env Redactor,
    log: &'env Sink,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: String,
}

#[derive(Clone, Default)]
pub(crate) struct Wire {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Arc<CompactRequest>>>>,
    events: Events,
}

impl Wire {
    pub(crate) fn new(events: &Events) -> Self {
        Self {
            replies: Arc::default(),
            requests: Arc::default(),
            events: events.clone(),
        }
    }

    pub(crate) fn queue(&self, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Reply {
                status,
                body: body.to_owned(),
            });
    }

    pub(crate) fn repeat(&self, status: u16, body: &str, times: u8) {
        for _ in 0..times {
            self.queue(status, body);
        }
    }

    pub(crate) fn requests(&self) -> Vec<Arc<CompactRequest>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CompactTransport for Wire {
    fn default_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn send(
        &self,
        request: CompactRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.events.push("send");
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::new(request));
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(reply) = reply else {
            panic!("the test must queue a reply per request");
        };
        ready(Ok(UpstreamResponse {
            status: reply.status,
            headers: http::HeaderMap::new(),
            body: reply.body.into_bytes(),
        }))
    }
}

#[derive(Clone)]
pub(crate) struct Source {
    wire: Wire,
    created: Arc<Mutex<Vec<(Option<bool>, bool)>>>,
}

impl Source {
    pub(crate) fn new(wire: &Wire) -> Self {
        Self {
            wire: wire.clone(),
            created: Arc::default(),
        }
    }

    pub(crate) fn created(&self) -> Vec<(Option<bool>, bool)> {
        self.created
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
    ) -> Result<Self::Client, ClientError> {
        self.created
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((ssl_verify, shared_session.is_some()));
        Ok(self.wire.clone())
    }
}

pub(crate) struct TestConfig {
    pub(crate) events: Events,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) supported: Option<Vec<String>>,
    pub(crate) dropped: Arc<Mutex<Vec<bool>>>,
}

impl TestConfig {
    pub(crate) fn new(events: &Events, supported: Option<Vec<String>>) -> Self {
        Self {
            events: events.clone(),
            api_key: Some(SecretString::from("sk-provider")),
            supported,
            dropped: Arc::default(),
        }
    }
}

struct NoIdentity;

impl WorkloadIdentity for NoIdentity {
    fn bearer_token(
        &mut self,
        _api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Option<SecretString> {
        None
    }
}

impl CompactConfig for TestConfig {
    fn validate_environment(
        &self,
        headers: Headers,
        _model: &str,
        _params: &CompactParams,
    ) -> Result<Headers, ConfigFailure> {
        let mut identity = NoIdentity;
        Ok(validate_environment(
            headers,
            ApiKeySources {
                litellm_params: None,
                litellm: None,
                openai: None,
                environment: self.api_key.as_ref(),
            },
            None,
            ResponsesProvider::OpenAi,
            &mut identity,
        ))
    }

    fn complete_url(&self, params: &CompactParams) -> Result<String, ConfigFailure> {
        Ok(get_complete_url(
            params.api_base.as_deref(),
            None,
            None,
            None,
        ))
    }

    fn transform_compact_request(
        &self,
        request: &CompactTransformRequest<'_>,
    ) -> Result<ResourceRequest, ConfigFailure> {
        let provider_request = ProviderCompactRequest {
            model: request.model,
            input: request.input,
            options: request.optional_params,
            provider: ProviderId::Openai,
            model_info: None,
        };
        compact_request(&provider_request, request.api_base).map_err(ConfigFailure::new)
    }

    fn normalize_request(&self, data: Object) -> Result<Object, ConfigFailure> {
        Ok(data)
    }

    fn sign_request(&self, request: &SignRequest<'_>) -> Result<Signed, ConfigFailure> {
        self.events.push("sign");
        Ok(Signed {
            headers: request.headers.clone(),
            body: None,
        })
    }

    fn sign_request_off_loop(
        &self,
        request: &SignRequest<'_>,
    ) -> impl Future<Output = Result<Signed, ConfigFailure>> + Send {
        self.events.push("off_loop");
        ready(self.sign_request(request))
    }

    fn transform_compact_response(
        &self,
        reply: &RawResponse<'_>,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<ResponsesReply, ConfigFailure> {
        self.events.push("response");
        transform_response(reply, env.now.timestamp(), state, env).map_err(ConfigFailure::new)
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

impl ResponsesParamProvider for TestConfig {
    type Error = CompactParamError;

    fn name(&self) -> &str {
        "openai"
    }

    fn supported_params(&self, _model: &str) -> Result<Option<Vec<String>>, Self::Error> {
        Ok(self.supported.clone())
    }

    fn map_params(
        &self,
        params: &Params,
        _model: &str,
        drop_params: bool,
    ) -> Result<Params, Self::Error> {
        self.dropped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(drop_params);
        let Some(supported) = &self.supported else {
            return Ok(params.clone());
        };
        if !drop_params {
            return Ok(params.clone());
        }
        Ok(params
            .iter()
            .filter(|(name, _value)| supported.iter().any(|allowed| allowed == *name))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect())
    }
}

pub(crate) struct Configs {
    pub(crate) events: Events,
    pub(crate) supported: Option<Vec<String>>,
}

// The registry decides native support; the factory builds a config for whatever B157 let through
impl CompactConfigFactory for Configs {
    type Provider = TestConfig;

    fn compact_config(&self, _request: &ConfigRequest<'_>) -> Option<TestConfig> {
        Some(TestConfig::new(&self.events, self.supported.clone()))
    }
}

pub(crate) struct Registry;

impl ConfigLookup for Registry {
    fn responses_api_config(
        &self,
        _model: &str,
        provider: &str,
        _api_base: Option<&str>,
    ) -> Option<NativeConfig> {
        matches!(provider, "openai" | "azure" | "xai").then_some(NativeConfig {
            encrypted_agent_messages: true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolution {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) api_base: Option<String>,
}

#[derive(Default)]
pub(crate) struct ModelNames {
    seen: Mutex<Vec<Resolution>>,
}

impl ModelNames {
    pub(crate) fn seen(&self) -> Vec<Resolution> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ModelResolver for ModelNames {
    fn resolve_model(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Resolution {
                model: request.model.to_owned(),
                custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
                api_base: request.api_base.map(str::to_owned),
            });
        let (model, provider) = match request.custom_llm_provider {
            Some(given) => (request.model.to_owned(), given.to_owned()),
            None => {
                let Some((prefix, rest)) = request.model.split_once('/') else {
                    return Err(ResolutionError::MissingProvider {
                        model: request.model.to_owned(),
                    });
                };
                let provider = match prefix {
                    "none" => String::new(),
                    other => other.to_owned(),
                };
                (rest.to_owned(), provider)
            }
        };
        Ok(ResolvedModel {
            model,
            provider: ProviderIdentity::Custom(provider),
            api_base: None,
            api_key: None,
        })
    }
}

pub(crate) fn call(model: &str, input: ResponsesInput) -> CompactResponsesCall {
    CompactResponsesCall {
        model: model.to_owned(),
        input,
        instructions: None,
        previous_response_id: None,
        custom_llm_provider: None,
        api_key: None,
        api_base: Some(String::from("http://mock.test/v1")),
        ssl_verify: None,
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        timeout: None,
        shared_session: None,
        kwargs: IndexMap::new(),
        acompact_responses: None,
    }
}

pub(crate) fn text_input(text: &str) -> ResponsesInput {
    ResponsesInput::Text(text.to_owned())
}

pub(crate) fn item_input(text: &str) -> ResponsesInput {
    let parsed: PromptJson = serde_json::from_str(text).expect("the item input is JSON");
    let PromptJson::List(items) = parsed else {
        panic!("the item input must be a list");
    };
    ResponsesInput::Items(items.into_iter().map(ResponsesItem).collect())
}

pub(crate) fn kwargs(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the kwargs are JSON")
}

pub(crate) fn env_vars(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn json(text: &str) -> PromptJson {
    serde_json::from_str(text).expect("the expected JSON parses")
}

pub(crate) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the expected object parses")
}

pub(crate) fn body_json(request: &CompactRequest) -> PromptJson {
    let RequestBody::Json(bytes) = &request.body else {
        panic!("the compact request must carry a JSON body");
    };
    serde_json::from_slice(bytes).expect("the compact request body is JSON")
}

pub(crate) fn typed_id(reply: &ResponsesReply) -> String {
    let ReplyBody::Typed(response) = &reply.response else {
        panic!("the compact reply must decode to the typed response");
    };
    response.id.clone()
}

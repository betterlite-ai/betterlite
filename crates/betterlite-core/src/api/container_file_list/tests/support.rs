#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::openai_credentials::Settings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::{BaseLlmException, ContainerError};
use betterlite_openai::api::container_contracts::records::{
    ContainerFileListResponse, ContainerListResponse, ContainerObject, DeleteContainerResult,
};
use betterlite_openai::containers::OpenAiContainerConfig;
use betterlite_openai::containers::container_file_http::PreCallHook;
use betterlite_openai::containers::container_file_http::request::{ParamValue, SslVerify};
use betterlite_openai::containers::container_file_http::transport::{
    AsyncClient, BlockingClient, ClientSource, ContainerHttpReply, ContainerHttpRequest,
    TransferError,
};
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use tokio::net::TcpListener;
use url::Url;

use super::error::{ContainerFileListFailure, FileListHandlerError};
use super::execution::ContainerFileListCall;
use super::execution::blocking::container_file_list_handler;
use super::{ContainerFileListDeps, ContainerFileListRequest, list_container_files};
use crate::api::container_file_endpoints::handler::ContainerConfigLookup;

pub(crate) const CALL_ID: &str = "call-1";
pub(crate) const API_KEY: &str = "sk-test";
pub(crate) const API_BASE: &str = "https://api.openai.com/v1";
pub(crate) const FILES_BASE: &str = "https://api.openai.com/v1/containers";
pub(crate) const CONTAINER_ID: &str = "cntr_original";
pub(crate) const REQUEST_TIMEOUT: f64 = 1234.0;
pub(crate) const INITIAL_CALL_TYPE: &str = "endpoint_func";
pub(crate) const FILE_LIST_BODY: &str = r#"{"object":"list","data":[{"id":"cfile_1","object":"container.file","container_id":"cntr_original","bytes":12,"created_at":3,"path":"/mnt/data/a.txt","source":"user"}],"first_id":"cfile_1","last_id":null,"has_more":false}"#;

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

pub(crate) fn logging() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "container".to_owned(),
            call_type: INITIAL_CALL_TYPE.to_owned(),
            start_time: moment(),
            litellm_call_id: CALL_ID.to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
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
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

pub(crate) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the fixture object parses")
}

#[derive(Debug)]
struct ProviderCost;

impl ContainerCosts for ProviderCost {
    fn code_interpreter_cost(&self, _sessions: u32, _provider: &str) -> Decimal {
        Decimal::ZERO
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EnvironmentCall {
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) api_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BaseCall {
    pub(crate) api_base: Option<String>,
    pub(crate) params: Object,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TransformCall {
    pub(crate) container_id: String,
    pub(crate) api_base: String,
    pub(crate) params: Object,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) after: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<String>,
    pub(crate) extra_query: Option<Object>,
}

// The real OpenAI container config, recording the four inputs the handler feeds it
pub(crate) struct RecordingConfig {
    inner: OpenAiContainerConfig,
    environment: Mutex<Vec<EnvironmentCall>>,
    bases: Mutex<Vec<BaseCall>>,
    transforms: Mutex<Vec<TransformCall>>,
}

impl RecordingConfig {
    pub(crate) fn new(api_base: &str) -> Self {
        Self {
            inner: OpenAiContainerConfig {
                settings: Settings {
                    api_base: api_base.to_owned(),
                    organization: None,
                },
                globals: LiteLlmGlobals::default(),
                env_api_key: None,
                costs: Arc::new(ProviderCost),
            },
            environment: Mutex::default(),
            bases: Mutex::default(),
            transforms: Mutex::default(),
        }
    }

    pub(crate) fn environment(&self) -> Vec<EnvironmentCall> {
        self.environment
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn bases(&self) -> Vec<BaseCall> {
        self.bases
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn transforms(&self) -> Vec<TransformCall> {
        self.transforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[rustfmt::skip]
impl BaseContainerConfig for RecordingConfig {
    fn get_supported_openai_params(&self) -> Vec<String> { self.inner.get_supported_openai_params() }

    fn map_openai_params(&self, container_create_optional_params: &Object, drop_params: bool) -> Object {
        self.inner.map_openai_params(container_create_optional_params, drop_params)
    }

    fn validate_environment(&self, headers: &BTreeMap<String, String>, api_key: Option<&str>) -> BTreeMap<String, String> {
        self.environment.lock().unwrap_or_else(PoisonError::into_inner).push(EnvironmentCall {
            headers: headers.clone(),
            api_key: api_key.map(str::to_owned),
        });
        self.inner.validate_environment(headers, api_key)
    }

    fn get_complete_url(&self, api_base: Option<&str>, params: &Object) -> Result<String, ContainerError> {
        self.bases.lock().unwrap_or_else(PoisonError::into_inner).push(BaseCall {
            api_base: api_base.map(str::to_owned),
            params: params.clone(),
        });
        self.inner.get_complete_url(api_base, params)
    }

    fn transform_container_create_request(&self, name: &str, container_create_optional_request_params: &Object, context: ContainerRequestContext<'_>) -> Object {
        self.inner.transform_container_create_request(name, container_create_optional_request_params, context)
    }

    fn transform_container_create_response(&self, raw_response: &[u8]) -> Result<ContainerObject, ContainerError> {
        self.inner.transform_container_create_response(raw_response)
    }

    fn transform_container_list_request(&self, api_base: &str, context: ContainerRequestContext<'_>, query: &ContainerListQuery<'_>) -> (String, Object) {
        self.inner.transform_container_list_request(api_base, context, query)
    }

    fn transform_container_list_response(&self, raw_response: &[u8]) -> Result<ContainerListResponse, ContainerError> {
        self.inner.transform_container_list_response(raw_response)
    }

    fn transform_container_retrieve_request(&self, container_id: &str, api_base: &str, context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> {
        self.inner.transform_container_retrieve_request(container_id, api_base, context)
    }

    fn transform_container_retrieve_response(&self, raw_response: &[u8]) -> Result<ContainerObject, ContainerError> {
        self.inner.transform_container_retrieve_response(raw_response)
    }

    fn transform_container_delete_request(&self, container_id: &str, api_base: &str, context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> {
        self.inner.transform_container_delete_request(container_id, api_base, context)
    }

    fn transform_container_delete_response(&self, raw_response: &[u8]) -> Result<DeleteContainerResult, ContainerError> {
        self.inner.transform_container_delete_response(raw_response)
    }

    fn transform_container_file_list_request(&self, container_id: &str, api_base: &str, context: ContainerRequestContext<'_>, query: &ContainerListQuery<'_>) -> Result<(String, Object), ContainerError> {
        self.transforms.lock().unwrap_or_else(PoisonError::into_inner).push(TransformCall {
            container_id: container_id.to_owned(),
            api_base: api_base.to_owned(),
            params: context.litellm_params.clone(),
            headers: context.headers.clone(),
            after: query.after.map(str::to_owned),
            limit: query.limit,
            order: query.order.map(str::to_owned),
            extra_query: query.extra_query.cloned(),
        });
        self.inner.transform_container_file_list_request(container_id, api_base, context, query)
    }

    fn transform_container_file_list_response(&self, raw_response: &[u8]) -> Result<ContainerFileListResponse, ContainerError> {
        self.inner.transform_container_file_list_response(raw_response)
    }

    fn transform_container_file_content_request(&self, container_id: &str, file_id: &str, api_base: &str, context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> {
        self.inner.transform_container_file_content_request(container_id, file_id, api_base, context)
    }

    fn transform_container_file_content_response(&self, raw_response: &[u8]) -> Result<Vec<u8>, ContainerError> {
        self.inner.transform_container_file_content_response(raw_response)
    }

    fn get_error_class(&self, error_message: &str, status_code: u16, headers: &BTreeMap<String, String>) -> BaseLlmException {
        self.inner.get_error_class(error_message, status_code, headers)
    }
}

// B3361's provider config manager, behind the caller's own registry
pub(crate) struct Lookup {
    config: Option<RecordingConfig>,
    providers: Mutex<Vec<ProviderId>>,
}

impl Lookup {
    pub(crate) fn answering() -> Self {
        Self {
            config: Some(RecordingConfig::new(API_BASE)),
            providers: Mutex::default(),
        }
    }

    pub(crate) fn empty() -> Self {
        Self {
            config: None,
            providers: Mutex::default(),
        }
    }

    pub(crate) fn providers(&self) -> Vec<ProviderId> {
        self.providers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn config(&self) -> &RecordingConfig {
        self.config
            .as_ref()
            .expect("the fixture holds a provider config")
    }
}

impl ContainerConfigLookup for Lookup {
    fn container_config(&self, provider: ProviderId) -> Option<&dyn BaseContainerConfig> {
        self.providers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(provider);
        let config: &dyn BaseContainerConfig = self.config.as_ref()?;
        Some(config)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Answer {
    Reply { status: u16, body: Vec<u8> },
    Failure,
    Deadline(Arc<Mutex<Option<TransferError>>>),
}

// Answers every request the same way, keeps what it was asked, and notes when it sent
#[derive(Clone)]
pub(crate) struct FakeClient {
    pub(crate) seen: Arc<Mutex<Vec<ContainerHttpRequest>>>,
    order: Option<Arc<Mutex<Vec<String>>>>,
    answer: Answer,
}

impl FakeClient {
    pub(crate) fn answering(status: u16, body: &str) -> Self {
        Self {
            seen: Arc::default(),
            order: None,
            answer: Answer::Reply {
                status,
                body: body.as_bytes().to_vec(),
            },
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            seen: Arc::default(),
            order: None,
            answer: Answer::Failure,
        }
    }

    pub(crate) fn deadline(error: TransferError) -> Self {
        Self {
            seen: Arc::default(),
            order: None,
            answer: Answer::Deadline(Arc::new(Mutex::new(Some(error)))),
        }
    }

    pub(crate) fn with_order(mut self, order: Arc<Mutex<Vec<String>>>) -> Self {
        self.order = Some(order);
        self
    }

    pub(crate) fn requests(&self) -> Vec<ContainerHttpRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn respond(&self, request: &ContainerHttpRequest) -> Result<ContainerHttpReply, TransferError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        if let Some(order) = &self.order {
            order
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push("send".to_owned());
        }
        match &self.answer {
            Answer::Reply { status, body } => Ok(ContainerHttpReply {
                status: *status,
                headers: BTreeMap::new(),
                body: body.clone(),
            }),
            Answer::Failure => Err(TransferError::Transport(TransportError::Closed)),
            Answer::Deadline(deadline) => {
                let error = deadline
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                    .expect("the deadline is handed out once");
                Err(error)
            }
        }
    }
}

impl BlockingClient for FakeClient {
    fn send(&self, request: &ContainerHttpRequest) -> Result<ContainerHttpReply, TransferError> {
        self.respond(request)
    }
}

impl AsyncClient for FakeClient {
    fn send(
        &self,
        request: &ContainerHttpRequest,
    ) -> impl Future<Output = Result<ContainerHttpReply, TransferError>> + Send {
        ready(self.respond(request))
    }
}

// Hands out clones of one client and counts how often each kind was asked for
pub(crate) struct FakeSource {
    pub(crate) client: FakeClient,
    pub(crate) blocking_made: Mutex<Vec<Option<SslVerify>>>,
    pub(crate) async_made: Mutex<Vec<Option<SslVerify>>>,
}

impl FakeSource {
    pub(crate) fn over(client: FakeClient) -> Self {
        Self {
            client,
            blocking_made: Mutex::default(),
            async_made: Mutex::default(),
        }
    }

    pub(crate) fn blocking_ssl(&self) -> Vec<Option<SslVerify>> {
        self.blocking_made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn async_ssl(&self) -> Vec<Option<SslVerify>> {
        self.async_made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for FakeSource {
    type Blocking = FakeClient;
    type Asynchronous = FakeClient;

    fn blocking_client(&self, ssl_verify: Option<&SslVerify>) -> Result<FakeClient, PoolError> {
        self.blocking_made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify.cloned());
        Ok(self.client.clone())
    }

    fn async_client(&self, ssl_verify: Option<&SslVerify>) -> Result<FakeClient, PoolError> {
        self.async_made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify.cloned());
        Ok(self.client.clone())
    }
}

const DEADLINE_LIMITS: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(100),
    ..TransportLimits::DEFAULT
};

// a real read timeout: a bound listener that never answers, and the client's own read limit ends the wait
pub(crate) async fn deadline_failure() -> TransferError {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    let client = ProxiedClient::build(base_builder().http1_only(), &policy, DEADLINE_LIMITS)
        .expect("the client");
    let request = OutboundRequest {
        method: Method::GET,
        url: Url::parse(&format!("http://{address}/")).expect("the URL"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: DEADLINE_LIMITS.read_timeout,
    };
    let failure = client
        .send_buffered(request)
        .await
        .expect_err("the answer never comes");
    TransferError::Transport(failure)
}

// what pre_call saw, and the logging state as it stood when the hook ran
#[derive(Clone)]
pub(crate) struct SeenPreCall {
    pub(crate) call_type: String,
    pub(crate) provider: Option<String>,
    pub(crate) input: Json,
    pub(crate) args: AdditionalArgs,
}

pub(crate) struct RecordingHook {
    seen: Mutex<Vec<SeenPreCall>>,
    fail: bool,
    order: Option<Arc<Mutex<Vec<String>>>>,
}

impl RecordingHook {
    pub(crate) fn passing() -> Self {
        Self {
            seen: Mutex::default(),
            fail: false,
            order: None,
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            fail: true,
            ..Self::passing()
        }
    }

    pub(crate) fn with_order(mut self, order: Arc<Mutex<Vec<String>>>) -> Self {
        self.order = Some(order);
        self
    }

    pub(crate) fn calls(&self) -> Vec<SeenPreCall> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PreCallHook for RecordingHook {
    fn pre_call(&self, state: &mut LoggingState, call: &PreCall) -> Result<(), PrePostError> {
        if let Some(order) = &self.order {
            order
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push("pre_call".to_owned());
        }
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SeenPreCall {
                call_type: state.call_type.clone(),
                provider: state.custom_llm_provider.clone(),
                input: call.input.clone(),
                args: call.additional_args.clone(),
            });
        if self.fail {
            return Err(PrePostError::RawRequestCaptured);
        }
        Ok(())
    }
}

pub(crate) struct Fixture {
    pub(crate) container_id: String,
    pub(crate) after: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<String>,
    pub(crate) timeout: Option<f64>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_query: Option<IndexMap<String, ParamValue>>,
    pub(crate) extra_body: Option<IndexMap<String, JsonValue>>,
    pub(crate) kwargs: IndexMap<String, Json>,
    pub(crate) logging: Mutex<LoggingState>,
    pub(crate) redactor: Redactor,
    pub(crate) lookup: Lookup,
    pub(crate) source: FakeSource,
    pub(crate) hook: RecordingHook,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self::answering(200, FILE_LIST_BODY)
    }

    pub(crate) fn answering(status: u16, body: &str) -> Self {
        let mut kwargs = IndexMap::new();
        kwargs.insert("litellm_call_id".to_owned(), Json::Text(CALL_ID.to_owned()));
        Self {
            container_id: CONTAINER_ID.to_owned(),
            after: None,
            limit: None,
            order: None,
            timeout: None,
            api_key: None,
            api_base: None,
            api_version: None,
            provider: None,
            extra_headers: None,
            extra_query: None,
            extra_body: None,
            kwargs,
            logging: Mutex::new(logging()),
            redactor: redactor(),
            lookup: Lookup::answering(),
            source: FakeSource::over(FakeClient::answering(status, body)),
            hook: RecordingHook::passing(),
        }
    }

    pub(crate) fn config(&self) -> &RecordingConfig {
        self.lookup.config()
    }

    pub(crate) fn requests(&self) -> Vec<ContainerHttpRequest> {
        self.source.client.requests()
    }

    pub(crate) fn request(&self) -> ContainerFileListRequest<'_> {
        ContainerFileListRequest {
            container_id: &self.container_id,
            after: self.after.as_deref(),
            limit: self.limit,
            order: self.order.as_deref(),
            timeout: self.timeout,
            api_key: self.api_key.clone(),
            api_base: self.api_base.as_deref(),
            api_version: self.api_version.as_deref(),
            custom_llm_provider: self.provider.as_deref(),
            extra_headers: self.extra_headers.as_ref(),
            extra_query: self.extra_query.as_ref(),
            extra_body: self.extra_body.as_ref(),
            kwargs: &self.kwargs,
        }
    }

    pub(crate) fn deps(&self) -> ContainerFileListDeps<'_> {
        ContainerFileListDeps {
            logging: &self.logging,
            redactor: &self.redactor,
            request_timeout_seconds: REQUEST_TIMEOUT,
        }
    }

    pub(crate) fn call(&self) -> Result<ContainerFileListResponse, ContainerFileListFailure> {
        let request = self.request();
        list_container_files(
            &self.source,
            &self.hook,
            &self.lookup,
            self.deps(),
            &request,
        )
    }

    pub(crate) fn params(&self) -> Object {
        let mut params = Object::new();
        for (name, value) in &self.kwargs {
            params.insert(name.clone(), json!(value));
        }
        if let Some(key) = &self.api_key {
            params.insert("api_key".to_owned(), json!(key.expose_secret()));
        }
        if let Some(base) = &self.api_base {
            params.insert("api_base".to_owned(), json!(base));
        }
        if let Some(version) = &self.api_version {
            params.insert("api_version".to_owned(), json!(version));
        }
        params
    }

    pub(crate) fn handler_call<'call, Source>(
        &'call self,
        params: &'call Object,
        blocking: Option<&'call Source::Blocking>,
        asynchronous: Option<&'call Source::Asynchronous>,
    ) -> ContainerFileListCall<'call, Source>
    where
        Source: ClientSource,
    {
        ContainerFileListCall {
            config: self.config(),
            litellm_params: params,
            logging: &self.logging,
            extra_headers: self.extra_headers.as_ref(),
            extra_query: self.extra_query.as_ref(),
            container_id: &self.container_id,
            after: self.after.as_deref(),
            limit: self.limit,
            order: self.order.as_deref(),
            timeout_seconds: 600.0,
            blocking_client: blocking,
            async_client: asynchronous,
        }
    }

    pub(crate) fn handle(
        &self,
        params: &Object,
    ) -> Result<ContainerFileListResponse, FileListHandlerError> {
        container_file_list_handler(
            &self.source,
            &self.hook,
            &self.handler_call::<FakeSource>(params, None, None),
        )
    }
}

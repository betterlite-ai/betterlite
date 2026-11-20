#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::{BaseLlmException, ContainerError};
use betterlite_openai::api::container_contracts::records::{
    ContainerFileListResponse, ContainerListResponse, ContainerObject, DeleteContainerResult,
};
use betterlite_openai::containers::container_file_http::request::{ParamValue, SslVerify};
use betterlite_openai::containers::container_file_http::transport::{
    AsyncClient, BlockingClient, ClientSource, ContainerHttpReply, ContainerHttpRequest,
    TransferError,
};
use betterlite_openai::containers::container_file_http::{ContainerFileHttpError, PreCallHook};
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::Method;
use http::header::HeaderMap;
use indexmap::IndexMap;
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::time::sleep;
use url::Url;

use super::{ContainerResourceCall, ContainerResourceHandler, ContainerResourceHttpError, Handled};

pub(super) const CONTAINER_ID: &str = "cntr_1";
pub(super) const API_BASE: &str = "https://api.example/v1";
pub(super) const CONTAINER_JSON: &str = concat!(
    r#"{"id":"cntr_1","object":"container","created_at":1750000000,"#,
    r#""status":"running","name":"demo"}"#,
);
pub(super) const DELETED_JSON: &str =
    r#"{"id":"cntr_1","object":"container.deleted","deleted":true}"#;

pub(super) fn params(json: &str) -> Object {
    serde_json::from_str(json).expect("the params parse")
}

pub(super) fn container_object() -> ContainerObject {
    serde_json::from_str(CONTAINER_JSON).expect("the container document parses")
}

pub(super) fn deleted_container() -> DeleteContainerResult {
    serde_json::from_str(DELETED_JSON).expect("the deleted document parses")
}

pub(super) fn reply(status: u16, body: &str) -> ContainerHttpReply {
    ContainerHttpReply {
        status,
        headers: BTreeMap::from([("x-request-id".to_owned(), "req_1".to_owned())]),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn header_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn map_of(pairs: &[(String, String)]) -> BTreeMap<String, String> {
    pairs.iter().cloned().collect()
}

pub(super) fn query(pairs: &[(&str, ParamValue)]) -> IndexMap<String, ParamValue> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor")
}

pub(super) fn logging() -> Mutex<LoggingState> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"container","call_type":"fixture_call_type","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging init parses");
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated".to_owned(),
        Duration::default,
    )
    .expect("the logging state builds");
    Mutex::new(state)
}

pub(super) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the runtime")
}

pub(super) fn only(requests: &[ContainerHttpRequest]) -> ContainerHttpRequest {
    assert_eq!(requests.len(), 1, "the client sends exactly one request");
    requests.first().expect("one request").clone()
}

pub(super) fn done<Ready, Pending>(
    outcome: Result<Handled<Ready, Pending>, ContainerResourceHttpError>,
) -> Ready {
    match outcome.expect("the handler returns") {
        Handled::Done(ready) => ready,
        Handled::Pending(_) => panic!("the blocking handler hands back no future"),
    }
}

pub(super) fn refused<Value>(
    result: Result<Value, ContainerResourceHttpError>,
) -> ContainerResourceHttpError {
    match result {
        Ok(_) => panic!("the call is expected to fail"),
        Err(error) => error,
    }
}

fn split_error(
    error: ContainerResourceHttpError,
) -> Result<(BaseLlmException, bool), ContainerFileHttpError> {
    match error {
        ContainerResourceHttpError::Provider {
            exception,
            status_code_is_synthesized,
        } => Ok((exception, status_code_is_synthesized)),
        ContainerResourceHttpError::Call(call) => Err(call),
        ContainerResourceHttpError::QueryValue { name } => panic!("the {name} query value"),
    }
}

pub(super) fn provider_error(error: ContainerResourceHttpError) -> (BaseLlmException, bool) {
    match split_error(error) {
        Ok(provider) => provider,
        Err(call) => panic!("the call failed: {call}"),
    }
}

pub(super) fn call_error(error: ContainerResourceHttpError) -> ContainerFileHttpError {
    match split_error(error) {
        Ok((exception, _)) => panic!("the provider failed: {exception}"),
        Err(call) => call,
    }
}

pub(super) fn param_field(error: ContainerResourceHttpError) -> &'static str {
    match call_error(error) {
        ContainerFileHttpError::ParamType { field, .. } => field,
        error => panic!("the call failed: {error}"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TransformSeen {
    pub(super) container_id: String,
    pub(super) api_base: String,
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ErrorSeen {
    pub(super) message: String,
    pub(super) status: u16,
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Default)]
pub(super) struct StubConfig {
    pub(super) url: Option<String>,
    pub(super) transformed_url: Option<String>,
    pub(super) params: Object,
    pub(super) fail_transform: bool,
    seen: Mutex<Vec<TransformSeen>>,
    errors: Mutex<Vec<ErrorSeen>>,
    keys: Mutex<Vec<Option<String>>>,
    urls: Mutex<Vec<Option<String>>>,
}

impl StubConfig {
    pub(super) fn at(url: &str) -> Self {
        let mut config = Self::without_url();
        config.url = Some(url.to_owned());
        config
    }

    pub(super) fn without_url() -> Self {
        Self::default()
    }

    pub(super) fn seen(&self) -> Vec<TransformSeen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn errors(&self) -> Vec<ErrorSeen> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn keys(&self) -> Vec<Option<String>> {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn urls(&self) -> Vec<Option<String>> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn transform(
        &self,
        container_id: &str,
        api_base: &str,
        context: ContainerRequestContext<'_>,
    ) -> Result<(String, Object), ContainerError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(TransformSeen {
                container_id: container_id.to_owned(),
                api_base: api_base.to_owned(),
                headers: context.headers.clone(),
            });
        if self.fail_transform {
            return Err(ContainerError::ApiBaseRequired);
        }
        let url = self
            .transformed_url
            .clone()
            .unwrap_or_else(|| format!("{api_base}/containers/{container_id}"));
        Ok((url, self.params.clone()))
    }
}

#[rustfmt::skip]
impl BaseContainerConfig for StubConfig {
    fn get_supported_openai_params(&self) -> Vec<String> { Vec::new() }

    fn map_openai_params(&self, _params: &Object, _drop_params: bool) -> Object { Object::new() }

    fn validate_environment(&self, headers: &BTreeMap<String, String>, api_key: Option<&str>) -> BTreeMap<String, String> {
        self.keys.lock().unwrap_or_else(PoisonError::into_inner).push(api_key.map(str::to_owned));
        let mut resolved = headers.clone();
        if let Some(key) = api_key {
            resolved.insert("authorization".to_owned(), format!("Bearer {key}"));
        }
        resolved.insert("x-provider".to_owned(), "validated".to_owned());
        resolved
    }

    fn get_complete_url(&self, api_base: Option<&str>, _litellm_params: &Object) -> Result<String, ContainerError> {
        self.urls.lock().unwrap_or_else(PoisonError::into_inner).push(api_base.map(str::to_owned));
        self.url.clone().ok_or(ContainerError::ApiBaseRequired)
    }

    fn transform_container_create_request(&self, _name: &str, _params: &Object, _context: ContainerRequestContext<'_>) -> Object { Object::new() }

    fn transform_container_create_response(&self, _raw: &[u8]) -> Result<ContainerObject, ContainerError> { Err(ContainerError::Decode("unused".to_owned())) }

    fn transform_container_list_request(&self, _api_base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>) -> (String, Object) { (String::new(), Object::new()) }

    fn transform_container_list_response(&self, _raw: &[u8]) -> Result<ContainerListResponse, ContainerError> { Err(ContainerError::Decode("unused".to_owned())) }

    fn transform_container_retrieve_request(&self, container_id: &str, api_base: &str, context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> {
        self.transform(container_id, api_base, context)
    }

    fn transform_container_retrieve_response(&self, raw: &[u8]) -> Result<ContainerObject, ContainerError> {
        serde_json::from_slice(raw).map_err(|error| ContainerError::Decode(error.to_string()))
    }

    fn transform_container_delete_request(&self, container_id: &str, api_base: &str, context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> {
        self.transform(container_id, api_base, context)
    }

    fn transform_container_delete_response(&self, raw: &[u8]) -> Result<DeleteContainerResult, ContainerError> {
        serde_json::from_slice(raw).map_err(|error| ContainerError::Decode(error.to_string()))
    }

    fn transform_container_file_list_request(&self, _id: &str, _api_base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>) -> Result<(String, Object), ContainerError> { Ok((String::new(), Object::new())) }

    fn transform_container_file_list_response(&self, _raw: &[u8]) -> Result<ContainerFileListResponse, ContainerError> { Err(ContainerError::Decode("unused".to_owned())) }

    fn transform_container_file_content_request(&self, _id: &str, _file_id: &str, _api_base: &str, _context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> { Ok((String::new(), Object::new())) }

    fn transform_container_file_content_response(&self, _raw: &[u8]) -> Result<Vec<u8>, ContainerError> { Err(ContainerError::Decode("unused".to_owned())) }

    fn get_error_class(&self, error_message: &str, status_code: u16, headers: &BTreeMap<String, String>) -> BaseLlmException {
        self.errors.lock().unwrap_or_else(PoisonError::into_inner).push(ErrorSeen {
            message: error_message.to_owned(),
            status: status_code,
            headers: headers.clone(),
        });
        BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers: headers.clone(),
        }
    }
}

#[derive(Default)]
struct ClientState {
    replies: Mutex<VecDeque<Result<ContainerHttpReply, TransferError>>>,
    seen: Mutex<Vec<ContainerHttpRequest>>,
}

impl ClientState {
    fn answer(&self, reply: ContainerHttpReply) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Ok(reply));
    }

    fn fail(&self, error: TransferError) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Err(error));
    }

    fn next(&self, request: &ContainerHttpRequest) -> Result<ContainerHttpReply, TransferError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the fixture queued an answer")
    }

    fn seen(&self) -> Vec<ContainerHttpRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Default)]
pub(super) struct FakeClient {
    state: Arc<ClientState>,
}

impl FakeClient {
    pub(super) fn answer(&self, reply: ContainerHttpReply) {
        self.state.answer(reply);
    }

    pub(super) fn fail(&self, error: TransferError) {
        self.state.fail(error);
    }

    pub(super) fn seen(&self) -> Vec<ContainerHttpRequest> {
        self.state.seen()
    }
}

impl BlockingClient for FakeClient {
    fn send(&self, request: &ContainerHttpRequest) -> Result<ContainerHttpReply, TransferError> {
        self.state.next(request)
    }
}

impl AsyncClient for FakeClient {
    fn send(
        &self,
        request: &ContainerHttpRequest,
    ) -> impl Future<Output = Result<ContainerHttpReply, TransferError>> + Send {
        let state = Arc::clone(&self.state);
        let request = request.clone();
        async move { state.next(&request) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BuiltClient {
    pub(super) asynchronous: bool,
    pub(super) ssl_verify: Option<SslVerify>,
}

#[derive(Default)]
struct SourceState {
    built: Mutex<Vec<BuiltClient>>,
    failing: AtomicBool,
    client: Arc<ClientState>,
}

#[derive(Clone, Default)]
pub(super) struct FakeSource {
    state: Arc<SourceState>,
}

impl FakeSource {
    pub(super) fn answer(&self, reply: ContainerHttpReply) {
        self.state.client.answer(reply);
    }

    pub(super) fn fail(&self, error: TransferError) {
        self.state.client.fail(error);
    }

    pub(super) fn refuse(&self) {
        self.state.failing.store(true, Ordering::Relaxed);
    }

    pub(super) fn built(&self) -> Vec<BuiltClient> {
        self.state
            .built
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn seen(&self) -> Vec<ContainerHttpRequest> {
        self.state.client.seen()
    }

    fn build(
        &self,
        asynchronous: bool,
        ssl_verify: Option<&SslVerify>,
    ) -> Result<FakeClient, PoolError> {
        self.state
            .built
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(BuiltClient {
                asynchronous,
                ssl_verify: ssl_verify.cloned(),
            });
        if self.state.failing.load(Ordering::Relaxed) {
            return Err(PoolError::Closed);
        }
        Ok(FakeClient {
            state: Arc::clone(&self.state.client),
        })
    }
}

impl ClientSource for FakeSource {
    type Blocking = FakeClient;
    type Asynchronous = FakeClient;

    fn blocking_client(&self, ssl_verify: Option<&SslVerify>) -> Result<FakeClient, PoolError> {
        self.build(false, ssl_verify)
    }

    fn async_client(&self, ssl_verify: Option<&SslVerify>) -> Result<FakeClient, PoolError> {
        self.build(true, ssl_verify)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Logged {
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) params: BTreeMap<String, String>,
    pub(super) container_id: String,
}

#[derive(Default)]
struct HookState {
    seen: Mutex<Vec<Logged>>,
    failing: AtomicBool,
}

#[derive(Clone, Default)]
pub(super) struct RecordingHook {
    state: Arc<HookState>,
}

impl RecordingHook {
    pub(super) fn refuse(&self) {
        self.state.failing.store(true, Ordering::Relaxed);
    }

    pub(super) fn seen(&self) -> Vec<Logged> {
        self.state
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PreCallHook for RecordingHook {
    fn pre_call(&self, _state: &mut LoggingState, call: &PreCall) -> Result<(), PrePostError> {
        let args = &call.additional_args;
        let mut headers = BTreeMap::new();
        for (name, value) in args.headers.iter().flatten() {
            if let Json::Text(text) = value {
                headers.insert(name.clone(), text.clone());
            }
        }
        let mut params = BTreeMap::new();
        if let Some(Json::Object(logged)) = args.passthrough.get("params") {
            for (name, value) in logged {
                if let Json::Text(text) = value {
                    params.insert(name.clone(), text.clone());
                }
            }
        }
        let container_id = match args.passthrough.get("container_id") {
            Some(Json::Text(text)) => text.clone(),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => String::new(),
        };
        self.state
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Logged {
                url: args.api_base.clone().unwrap_or_default(),
                headers,
                params,
                container_id,
            });
        if self.state.failing.load(Ordering::Relaxed) {
            return Err(PrePostError::CallbackFailed {
                message: "the hook refused the call".to_owned(),
            });
        }
        Ok(())
    }
}

pub(super) fn closed_error() -> TransferError {
    TransferError::from(TransportError::Closed)
}

pub(super) fn rejected_timeout() -> TransferError {
    TransferError::from(TransportError::Rejected(HttpError::Timeout))
}

const LOCAL_TIMEOUT: Duration = Duration::from_millis(100);
const LOCAL_HOLD: Duration = Duration::from_millis(300);

pub(super) async fn timeout_error() -> TransferError {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    let client = ProxiedClient::build(
        base_builder(),
        &policy,
        TransportLimits {
            read_timeout: LOCAL_TIMEOUT,
            ..TransportLimits::DEFAULT
        },
    )
    .expect("the client");
    let request = OutboundRequest {
        method: Method::GET,
        url: Url::parse(&format!("http://{address}/containers/{CONTAINER_ID}")).expect("the URL"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: LOCAL_HOLD,
    };
    let held = async {
        let (mut stream, _) = listener.accept().await.expect("the connection");
        let mut chunk = [0_u8; 64];
        let read = stream.read(&mut chunk).await.expect("the request bytes");
        assert_ne!(read, 0, "the client sends its request");
        sleep(LOCAL_HOLD).await;
    };
    let (result, ()) = tokio::join!(client.send_buffered(request), held);
    TransferError::from(result.expect_err("the deadline ends the request"))
}

pub(super) struct Fixture {
    pub(super) config: StubConfig,
    pub(super) hook: RecordingHook,
    pub(super) supplied: FakeClient,
    pub(super) source: FakeSource,
    pub(super) logging: Mutex<LoggingState>,
    pub(super) litellm_params: Object,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            config: StubConfig::at(API_BASE),
            hook: RecordingHook::default(),
            supplied: FakeClient::default(),
            source: FakeSource::default(),
            logging: logging(),
            litellm_params: params(r#"{"api_key":"sk-test","api_base":"https://base.example"}"#),
        }
    }

    pub(super) fn handler(&self) -> ContainerResourceHandler<FakeSource, RecordingHook> {
        ContainerResourceHandler {
            clients: self.source.clone(),
            logging: self.hook.clone(),
        }
    }

    pub(super) fn call(&self) -> ContainerResourceCall<'_, FakeSource> {
        ContainerResourceCall {
            container_id: CONTAINER_ID,
            config: &self.config,
            litellm_params: &self.litellm_params,
            logging_obj: &self.logging,
            extra_headers: None,
            extra_query: None,
            timeout: Some(Duration::from_secs(6000)),
            blocking_client: None,
            async_client: None,
        }
    }
}

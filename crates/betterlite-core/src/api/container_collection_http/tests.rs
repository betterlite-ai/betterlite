#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
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
use betterlite_openai::containers::container_file_http::PreCallHook;
use betterlite_openai::containers::container_file_http::request::SslVerify;
use betterlite_openai::containers::container_file_http::transport::{
    ClientSource, ContainerHttpReply, TransferError,
};
use betterlite_openai::containers::{replies, requests};
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::Method;
use http::header::HeaderMap;
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::time::sleep;
use url::Url;

use super::{
    CollectionBase, CollectionReply, CollectionRequest, CollectionTransport,
    ContainerCollectionError, ContainerCreateCall, ContainerListCall,
};

pub(super) const BASE: &str = "https://api.example.com/v1/containers";
pub(super) const API_VERSION: &str = "2025-04-01-preview";
pub(super) const CREATE_BODY: &str = r#"{"id":"cntr_abc","object":"container","created_at":1700000000,"status":"running","expires_after":{"anchor":"last_active_at","minutes":20},"last_active_at":null,"name":"box"}"#;
pub(super) const LIST_BODY: &str = r#"{"object":"list","data":[{"id":"cntr_a","object":"container","created_at":1,"status":"running","expires_after":null,"last_active_at":1730000000,"name":null}],"first_id":"cntr_a","last_id":"cntr_a","has_more":false}"#;
pub(super) const REQUEST_HEADERS: &str = "x-request-id";
pub(super) const REQUEST_ID: &str = "req_1";

type EnvironmentCall = (BTreeMap<String, String>, Option<String>);
type BaseCall = (Option<String>, Object);
type CreateCall = (String, Object, BTreeMap<String, String>);
type ListCall = (String, BTreeMap<String, String>, Object);
type Prepared = (String, Object);
type Reply<T> = Result<T, ContainerError>;

// Records what the handler hands the provider config, and transforms through the real request and reply helpers
pub(super) struct StubConfig {
    pub(super) api_base: String,
    pub(super) environment: Mutex<Vec<EnvironmentCall>>,
    pub(super) bases: Mutex<Vec<BaseCall>>,
    pub(super) creates: Mutex<Vec<CreateCall>>,
    pub(super) lists: Mutex<Vec<ListCall>>,
}

impl StubConfig {
    pub(super) fn with_base(api_base: &str) -> Self {
        Self {
            api_base: api_base.to_owned(),
            environment: Mutex::default(),
            bases: Mutex::default(),
            creates: Mutex::default(),
            lists: Mutex::default(),
        }
    }
}

#[rustfmt::skip]
impl BaseContainerConfig for StubConfig {
    fn get_supported_openai_params(&self) -> Vec<String> { Vec::new() }

    fn map_openai_params(&self, _params: &Object, _drop_params: bool) -> Object {
        Object::new()
    }

    fn validate_environment(&self, headers: &BTreeMap<String, String>, api_key: Option<&str>) -> BTreeMap<String, String> {
        self.environment.lock().unwrap_or_else(PoisonError::into_inner).push((headers.clone(), api_key.map(str::to_owned)));
        let mut merged = headers.clone();
        merged.insert(String::from("Authorization"), format!("Bearer {}", api_key.unwrap_or("none")));
        merged
    }

    fn get_complete_url(&self, api_base: Option<&str>, litellm_params: &Object) -> Reply<String> {
        self.bases.lock().unwrap_or_else(PoisonError::into_inner).push((api_base.map(str::to_owned), litellm_params.clone()));
        Ok(self.api_base.clone())
    }

    fn transform_container_create_request(
        &self, name: &str, container_create_optional_request_params: &Object, context: ContainerRequestContext<'_>,
    ) -> Object {
        self.creates.lock().unwrap_or_else(PoisonError::into_inner).push((name.to_owned(), container_create_optional_request_params.clone(), context.headers.clone()));
        requests::create_request(name, container_create_optional_request_params)
    }

    fn transform_container_create_response(&self, raw_response: &[u8]) -> Reply<ContainerObject> {
        replies::decode_container_reply(raw_response)
    }

    fn transform_container_list_request(
        &self, api_base: &str, context: ContainerRequestContext<'_>, query: &ContainerListQuery<'_>,
    ) -> Prepared {
        let params = requests::list_query(query);
        self.lists.lock().unwrap_or_else(PoisonError::into_inner).push((api_base.to_owned(), context.headers.clone(), params.clone()));
        (format!("{api_base}?api-version={API_VERSION}"), params)
    }

    fn transform_container_list_response(&self, raw_response: &[u8]) -> Reply<ContainerListResponse> {
        replies::decode_container_reply(raw_response)
    }

    fn transform_container_retrieve_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Reply<Prepared> { Ok((String::new(), Object::new())) }

    fn transform_container_retrieve_response(&self, raw_response: &[u8]) -> Reply<ContainerObject> {
        replies::decode_container_reply(raw_response)
    }

    fn transform_container_delete_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Reply<Prepared> { Ok((String::new(), Object::new())) }

    fn transform_container_delete_response(&self, _raw: &[u8]) -> Reply<DeleteContainerResult> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_file_list_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>,
    ) -> Reply<Prepared> { Ok((String::new(), Object::new())) }

    fn transform_container_file_list_response(&self, _raw: &[u8]) -> Reply<ContainerFileListResponse> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_file_content_request(
        &self, _id: &str, _file: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Reply<Prepared> { Ok((String::new(), Object::new())) }

    fn transform_container_file_content_response(&self, _raw: &[u8]) -> Reply<Vec<u8>> {
        Err(ContainerError::ApiBaseRequired)
    }
}

type Events = Arc<Mutex<Vec<&'static str>>>;

// Answers every request with one reply or one failure, keeps what it was asked, and marks each send in the shared order
#[derive(Clone)]
pub(super) struct FakeClient {
    pub(super) requests: Arc<Mutex<Vec<CollectionRequest>>>,
    events: Events,
    reply: ContainerHttpReply,
    failure: Arc<Mutex<Option<TransferError>>>,
}

impl FakeClient {
    pub(super) fn answering(events: &Events, status: u16, body: &str) -> Self {
        Self::answering_bytes(events, status, body.as_bytes().to_vec())
    }

    pub(super) fn answering_bytes(events: &Events, status: u16, body: Vec<u8>) -> Self {
        Self {
            requests: Arc::default(),
            events: Arc::clone(events),
            reply: ContainerHttpReply {
                status,
                headers: BTreeMap::from([(REQUEST_HEADERS.to_owned(), REQUEST_ID.to_owned())]),
                body,
            },
            failure: Arc::default(),
        }
    }

    pub(super) fn failing(events: &Events, failure: TransferError) -> Self {
        let client = Self::answering(events, 200, CREATE_BODY);
        let mut held = client
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *held = Some(failure);
        drop(held);
        client
    }

    fn answer(&self, request: &CollectionRequest) -> CollectionReply {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("send");
        let stopped = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        stopped.map_or_else(|| Ok(self.reply.clone()), Err)
    }

    pub(super) fn seen(&self) -> Vec<CollectionRequest> {
        self.requests.lock().unwrap().clone()
    }
}

// The kind decides which client a handler may reach, so each kind is its own type
pub(super) struct BlockingFake(pub(super) FakeClient);

pub(super) struct AsyncFake(pub(super) FakeClient);

impl CollectionTransport for BlockingFake {
    fn send(&self, request: &CollectionRequest) -> impl Future<Output = CollectionReply> + Send {
        ready(self.0.answer(request))
    }
}

impl CollectionTransport for AsyncFake {
    fn send(&self, request: &CollectionRequest) -> impl Future<Output = CollectionReply> + Send {
        ready(self.0.answer(request))
    }
}

// Hands out one client per kind and counts how often each kind was asked for
pub(super) struct FakeSource {
    pub(super) blocking: FakeClient,
    pub(super) asynchronous: FakeClient,
    pub(super) blocking_made: Mutex<Vec<Option<SslVerify>>>,
    pub(super) async_made: Mutex<Vec<Option<SslVerify>>>,
}

impl FakeSource {
    pub(super) fn over(blocking: FakeClient, asynchronous: FakeClient) -> Self {
        Self {
            blocking,
            asynchronous,
            blocking_made: Mutex::default(),
            async_made: Mutex::default(),
        }
    }
}

impl ClientSource for FakeSource {
    type Blocking = BlockingFake;
    type Asynchronous = AsyncFake;

    fn blocking_client(&self, ssl_verify: Option<&SslVerify>) -> Result<BlockingFake, PoolError> {
        self.blocking_made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify.cloned());
        Ok(BlockingFake(self.blocking.clone()))
    }

    fn async_client(&self, ssl_verify: Option<&SslVerify>) -> Result<AsyncFake, PoolError> {
        self.async_made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify.cloned());
        Ok(AsyncFake(self.asynchronous.clone()))
    }
}

pub(super) struct RecordingHook {
    pub(super) seen: Mutex<Vec<(Json, AdditionalArgs)>>,
    pub(super) events: Events,
    pub(super) fail: bool,
}

impl RecordingHook {
    pub(super) fn passing(events: &Events) -> Self {
        Self {
            seen: Mutex::default(),
            events: Arc::clone(events),
            fail: false,
        }
    }

    pub(super) fn failing(events: &Events) -> Self {
        Self {
            seen: Mutex::default(),
            events: Arc::clone(events),
            fail: true,
        }
    }

    pub(super) fn calls(&self) -> Vec<(Json, AdditionalArgs)> {
        self.seen.lock().unwrap().clone()
    }
}

impl PreCallHook for RecordingHook {
    fn pre_call(&self, _state: &mut LoggingState, call: &PreCall) -> Result<(), PrePostError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((call.input.clone(), call.additional_args.clone()));
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("log");
        if self.fail {
            return Err(PrePostError::RawRequestCaptured);
        }
        Ok(())
    }
}

pub(super) fn logging_state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"container","call_type":"create_container","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated".to_owned(),
        Duration::default,
    )
    .unwrap()
}

// Everything one call borrows, owned in one place so a test changes only the part it is about
pub(super) struct Fixture {
    pub(super) config: StubConfig,
    pub(super) params: Object,
    pub(super) create_params: Object,
    pub(super) logging: Mutex<LoggingState>,
    pub(super) extra_headers: Option<BTreeMap<String, String>>,
    pub(super) events: Events,
    pub(super) source: FakeSource,
    pub(super) hook: RecordingHook,
}

impl Fixture {
    pub(super) fn new(status: u16, body: &str) -> Self {
        Self::with_bytes(status, body.as_bytes().to_vec())
    }

    // A reply body that is not text, for the error text a body's bytes produce
    pub(super) fn with_bytes(status: u16, body: Vec<u8>) -> Self {
        let events: Events = Arc::default();
        let client = FakeClient::answering_bytes(&events, status, body);
        Self::over(events, client)
    }

    pub(super) fn failing(failure: TransferError) -> Self {
        let events: Events = Arc::default();
        let client = FakeClient::failing(&events, failure);
        Self::over(events, client)
    }

    fn over(events: Events, client: FakeClient) -> Self {
        Self {
            source: FakeSource::over(client.clone(), client),
            hook: RecordingHook::passing(&events),
            events,
            config: StubConfig::with_base(BASE),
            params: Object::new(),
            create_params: Object::new(),
            logging: Mutex::new(logging_state()),
            extra_headers: None,
        }
    }

    pub(super) fn base(&self, timeout: Option<Duration>) -> CollectionBase<'_, FakeSource> {
        CollectionBase {
            config: &self.config,
            litellm_params: &self.params,
            logging: &self.logging,
            extra_headers: self.extra_headers.as_ref(),
            timeout,
            source: &self.source,
            blocking: None,
            asynchronous: None,
            hook: &self.hook,
        }
    }

    pub(super) fn create_call<'call>(
        &'call self,
        name: &'call str,
    ) -> ContainerCreateCall<'call, FakeSource> {
        ContainerCreateCall {
            base: self.base(None),
            name,
            params: &self.create_params,
        }
    }

    pub(super) fn list_call<'call>(
        &'call self,
        query: ContainerListQuery<'call>,
    ) -> ContainerListCall<'call, FakeSource> {
        ContainerListCall {
            base: self.base(None),
            query,
        }
    }

    pub(super) fn sent(&self) -> Vec<CollectionRequest> {
        self.source.blocking.seen()
    }

    pub(super) fn order(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }
}

// A failure the transport reports without a socket: the base URL cannot be parsed
pub(super) fn url_failure() -> TransferError {
    TransferError::Url {
        url: String::from("::"),
        source: url::ParseError::RelativeUrlWithoutBase,
    }
}

const LOCAL_TIMEOUT: Duration = Duration::from_millis(100);
const LOCAL_HOLD: Duration = Duration::from_millis(300);

fn local_client(limits: TransportLimits) -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    ProxiedClient::build(base_builder(), &policy, limits).expect("the client")
}

// A local listener holds the answer past a 100ms read timeout, so the transport raises a real deadline failure
pub(super) async fn deadline_failure() -> TransferError {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let client = local_client(TransportLimits {
        read_timeout: LOCAL_TIMEOUT,
        ..TransportLimits::DEFAULT
    });
    let url = Url::parse(&format!("http://{address}/v1/containers")).expect("the URL");
    let request = OutboundRequest {
        method: Method::GET,
        url,
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
    TransferError::Transport(result.expect_err("the deadline ends the request"))
}

pub(super) fn provider_error(failure: &ContainerCollectionError) -> &BaseLlmException {
    match failure {
        ContainerCollectionError::Provider(provider) => provider,
        other @ (ContainerCollectionError::Config(_)
        | ContainerCollectionError::Params(_)
        | ContainerCollectionError::Client(_)
        | ContainerCollectionError::Logging(_)
        | ContainerCollectionError::Encode(_)
        | ContainerCollectionError::Suspended) => {
            panic!("expected a provider failure, got {other:?}")
        }
    }
}

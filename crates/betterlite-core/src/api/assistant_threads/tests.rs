#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::azure_endpoints;
use betterlite_config::types::ProxyTrust;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_models::decisions::Json;
use betterlite_openai::assistants::client::SdkClient;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::records::{
    MessageContent, MessageRole, ThreadCreate, ThreadMessageData, ThreadRecord,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;

use super::error::{ThreadError, ThreadFailure, sdk_class};
use super::params::{
    ThreadExtraBody, ThreadParams, ThreadTimeout, ThreadTimeoutValue, forwarded_azure_timeout,
    resolve_timeout,
};
use super::runtime::ThreadRuntime;
use super::target::{Operation, PrepareRequest, PreparedThread, Target, create_body};
use super::{
    AssistantThreads, CreateThreadCall, FacadeCall, GetThreadCall, SuppliedClient,
    ThreadFacade as _, ThreadOperation, execute,
};
use crate::api::resource_sdk_settings::ResourceSdkSettings;
use crate::api::resource_sdk_settings::timeout::{StructuredTimeout, TimeoutError};

const THREAD: &str = r#"{"id":"thread_a","object":"thread","created_at":1,"metadata":{"tag":"x"},"tool_resources":{}}"#;
const TIMEOUT_LIMITS: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(100),
    ..TransportLimits::DEFAULT
};
const TYPED_404: &str = r#"{"error":{"message":"missing","type":"invalid_request_error","param":"thread_id","code":"missing"}}"#;
const UNTYPED_404: &str = r#"{"error":{"message":"missing","param":"thread_id","code":"missing"}}"#;

#[derive(Debug, Clone, PartialEq)]
struct Seen {
    method: String,
    url: String,
    authorization: Option<String>,
    api_key: Option<String>,
    beta: Option<String>,
    deadline: f64,
    body: Json,
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Seen>>,
    replies: Mutex<VecDeque<(u16, String)>>,
}

impl Recorder {
    fn queue(&self, status: u16, text: &str) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back((status, text.to_owned()));
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn header(request: &OutboundRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn body_of(body: &RequestBody) -> Json {
    match body {
        RequestBody::Json(bytes) => serde_json::from_slice(bytes).unwrap_or(Json::Null),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            Json::Null
        }
    }
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let seen = Seen {
            method: request.method.as_str().to_owned(),
            url: url.to_string(),
            authorization: header(&request, "authorization"),
            api_key: header(&request, "api-key"),
            beta: header(&request, "openai-beta"),
            deadline: request.deadline.as_secs_f64(),
            body: body_of(&request.body),
        };
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
        let (status, text) = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the test queued a reply");
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: text.into_bytes(),
        }))
    }
}

#[derive(Default)]
struct Capture {
    calls: Mutex<Vec<String>>,
}

impl<T> ThreadRuntime<T> for Capture {
    fn run(
        &self,
        prepared: PreparedThread<'_, T>,
    ) -> impl Future<Output = Result<Option<ThreadRecord>, ThreadError>> + Send {
        let label = match &prepared.target {
            Target::Openai(client) => format!("openai:{:?}", client.kind),
            Target::Azure { client, flag, .. } => format!("azure:{:?}:{flag:?}", client.kind),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(label);
        ready(Ok(None))
    }
}

struct Harness {
    recorder: Arc<Recorder>,
    resources: ResourceSdkSettings,
    azure: azure_endpoints::Settings,
    env: BTreeMap<String, String>,
    sources: NoSources,
    redactor: Redactor,
}

impl Harness {
    fn new() -> Self {
        let mut resources = empty_resources();
        resources.openai_base_url = Some(String::from("https://example.test/v1"));
        resources.openai_api_key = Some(SecretString::from("sk-test"));
        resources.azure_api_base = Some(String::from("https://azure.test"));
        resources.azure_api_version = Some(String::from("2024-02-01"));
        resources.azure_openai_api_key = Some(SecretString::from("azure-key"));
        Self {
            recorder: Arc::default(),
            resources,
            azure: azure_endpoints::Settings::default(),
            env: BTreeMap::new(),
            sources: NoSources,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a redactor"),
        }
    }

    fn threads(&self) -> AssistantThreads<'_, Recorder> {
        AssistantThreads {
            transport: Arc::clone(&self.recorder),
            resources: &self.resources,
            azure: &self.azure,
            environment: &self.env,
            sources: &self.sources,
            redactor: &self.redactor,
            refresh_enabled: false,
            session: SessionKind::Dedicated,
        }
    }
}

const fn empty_resources() -> ResourceSdkSettings {
    ResourceSdkSettings {
        api_base: None,
        api_version: None,
        api_key: None,
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        openai_base_url: None,
        openai_api_base: None,
        openai_api_key: None,
        openai_organization: None,
        azure_api_base: None,
        azure_api_version: None,
        azure_openai_api_key: None,
        azure_api_key: None,
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}

fn message(text: &str) -> ThreadMessageData {
    ThreadMessageData {
        role: MessageRole::User,
        content: MessageContent::Text(text.to_owned()),
        attachments: None,
        metadata: None,
    }
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected value is JSON")
}

fn kwargs_of(text: &str) -> BTreeMap<String, Json> {
    serde_json::from_str(text).expect("the kwargs are JSON")
}

fn secret_text(token: &SecretString) -> String {
    token.expose_secret().to_owned()
}

// every field absent, the base the timeout tests change one field of at a time
const NO_PARAMS: ThreadParams = ThreadParams {
    api_key: None,
    api_base: None,
    api_version: None,
    organization: None,
    max_retries: None,
    timeout: None,
    request_timeout: None,
    azure_ad_token: None,
    tenant_id: None,
    client_id: None,
    client_secret: None,
    azure_username: None,
    azure_password: None,
    azure_scope: None,
    extra_body: ThreadExtraBody::Absent,
};

// the async wrapper's envelope names no provider fields besides the class and the status it derives
fn assert_sdk_failure(failure: &ThreadFailure, status: u16, class: &str) {
    let metadata = failure.class.metadata();
    assert_eq!(metadata.status_code, status);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    let sdk = failure.error.sdk_failure();
    assert_eq!(sdk.class, class);
    assert_eq!(sdk.status, status);
}

fn supplied_openai(transport: Arc<Recorder>) -> SdkClient<Recorder> {
    SdkClient {
        kind: ClientKind::Sync,
        api_key: Some(SecretString::from("supplied-key")),
        base_url: Some(String::from("https://supplied.test/v1")),
        timeout: Some(Duration::from_secs(42)),
        max_retries: Some(0),
        organization: None,
        transport,
    }
}

fn create_call(
    kwargs: &mut BTreeMap<String, Json>,
    messages: Option<Vec<ThreadMessageData>>,
) -> CreateThreadCall<'_, Recorder> {
    CreateThreadCall {
        custom_llm_provider: String::from("openai"),
        messages,
        metadata: Some(BTreeMap::from([("tag".to_owned(), json(r#""x""#))])),
        tool_resources: Some(json(r#"{"file_search":{"vector_store_ids":["vs_a"]}}"#)),
        client: None,
        kwargs,
    }
}

fn prepare_request(
    operation: Operation,
    kwargs: &BTreeMap<String, Json>,
    flag: Option<bool>,
) -> PrepareRequest<'_, Recorder> {
    PrepareRequest {
        provider: String::from("openai"),
        operation,
        client: None,
        kwargs,
        facade: ThreadOperation::Create,
        flag,
    }
}

const fn empty_create() -> Operation {
    Operation::Create(ThreadCreate {
        messages: None,
        metadata: None,
    })
}

#[tokio::test]
async fn create_posts_messages_and_metadata_only() {
    let harness = Harness::new();
    harness.recorder.queue(200, THREAD);
    let mut kwargs = BTreeMap::new();
    let thread = harness
        .threads()
        .create_thread(create_call(&mut kwargs, Some(vec![message("hello")])))
        .await
        .expect("the create succeeds")
        .expect("the mock returned a thread");
    assert_eq!(thread.id, "thread_a");
    let seen = &harness.recorder.seen()[0];
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.url, "https://example.test/v1/threads");
    assert_eq!(seen.authorization.as_deref(), Some("Bearer sk-test"));
    assert_eq!(seen.beta.as_deref(), Some("assistants=v2"));
    assert_eq!(
        seen.body,
        json(r#"{"messages":[{"role":"user","content":"hello"}],"metadata":{"tag":"x"}}"#)
    );
}

#[tokio::test]
async fn null_fields_are_omitted_and_empty_ones_remain() {
    let harness = Harness::new();
    harness.recorder.queue(200, THREAD);
    harness.recorder.queue(200, THREAD);
    let mut kwargs = BTreeMap::new();
    let mut call = create_call(&mut kwargs, None);
    call.metadata = None;
    call.tool_resources = None;
    let created = harness.threads().create_thread(call).await.expect("create");
    assert!(created.is_some());
    assert_eq!(harness.recorder.seen()[0].body, json("{}"));
    let mut empty_kwargs = BTreeMap::new();
    let mut empty_call = create_call(&mut empty_kwargs, Some(Vec::new()));
    empty_call.metadata = Some(BTreeMap::new());
    let empty_created = harness
        .threads()
        .create_thread(empty_call)
        .await
        .expect("create");
    assert!(empty_created.is_some());
    assert_eq!(
        harness.recorder.seen()[1].body,
        json(r#"{"messages":[],"metadata":{}}"#)
    );
}

#[tokio::test]
async fn retrieve_gets_the_thread_by_id() {
    let harness = Harness::new();
    harness.recorder.queue(200, THREAD);
    let mut kwargs = BTreeMap::new();
    kwargs.insert(String::from("api_key"), json(r#""ignored""#));
    let thread = harness
        .threads()
        .get_thread(GetThreadCall {
            custom_llm_provider: String::from("openai"),
            thread_id: String::from("thread_a"),
            client: None,
            kwargs: &mut kwargs,
        })
        .await
        .expect("the retrieve succeeds")
        .expect("the mock returned a thread");
    assert_eq!(thread.created_at, 1);
    let seen = &harness.recorder.seen()[0];
    assert_eq!(seen.method, "GET");
    assert_eq!(seen.url, "https://example.test/v1/threads/thread_a");
    assert_eq!(seen.body, Json::Null);
}

#[tokio::test]
async fn a_null_provider_result_passes_through() {
    let harness = Harness::new();
    let capture = Capture::default();
    let mut kwargs = BTreeMap::new();
    let thread = execute(
        &harness.threads(),
        FacadeCall::Create(create_call(&mut kwargs, None)),
        &capture,
    )
    .await
    .expect("a null provider result is not an error");
    assert!(thread.is_none());
    assert_eq!(capture.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_async_wrappers_run_once_and_move_their_flag() {
    let harness = Harness::new();
    harness.recorder.queue(200, THREAD);
    let mut kwargs = BTreeMap::new();
    let thread = harness
        .threads()
        .acreate_thread(create_call(&mut kwargs, Some(vec![message("hello")])))
        .await
        .expect("the create succeeds")
        .expect("the thread returns");
    assert_eq!(thread.id, "thread_a");
    assert_eq!(harness.recorder.seen().len(), 1);
    assert_eq!(kwargs.get("acreate_thread"), Some(&Json::Bool(true)));
    harness.recorder.queue(200, THREAD);
    let mut get_kwargs = BTreeMap::new();
    get_kwargs.insert(String::from("aget_thread"), Json::Bool(true));
    let retrieved = harness
        .threads()
        .aget_thread(GetThreadCall {
            custom_llm_provider: String::from("openai"),
            thread_id: String::from("thread_a"),
            client: None,
            kwargs: &mut get_kwargs,
        })
        .await
        .expect("the retrieve succeeds");
    assert!(retrieved.is_some());
    assert!(!get_kwargs.contains_key("aget_thread"));
}

#[tokio::test]
async fn a_supplied_openai_client_reaches_its_own_recorder_untouched() {
    let harness = Harness::new();
    let supplied = Arc::new(Recorder::default());
    supplied.queue(200, THREAD);
    supplied.queue(200, THREAD);
    let mut kwargs = BTreeMap::new();
    let mut call = create_call(&mut kwargs, None);
    call.client = Some(SuppliedClient::Openai(supplied_openai(Arc::clone(
        &supplied,
    ))));
    let created = harness
        .threads()
        .create_thread(call)
        .await
        .expect("the create succeeds");
    assert!(created.is_some());
    let mut get_kwargs = BTreeMap::new();
    let retrieved = harness
        .threads()
        .get_thread(GetThreadCall {
            custom_llm_provider: String::from("openai"),
            thread_id: String::from("thread_a"),
            client: Some(SuppliedClient::Openai(supplied_openai(Arc::clone(
                &supplied,
            )))),
            kwargs: &mut get_kwargs,
        })
        .await
        .expect("the retrieve succeeds");
    assert!(retrieved.is_some());
    let seen = supplied.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].url, "https://supplied.test/v1/threads");
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some("Bearer supplied-key")
    );
    assert_eq!(seen[0].deadline.to_bits(), 42.0_f64.to_bits());
    assert_eq!(seen[1].url, "https://supplied.test/v1/threads/thread_a");
    assert!(harness.recorder.seen().is_empty());
}

#[tokio::test]
async fn a_missing_api_key_maps_as_authentication_from_the_async_wrapper() {
    let mut harness = Harness::new();
    harness.resources.openai_api_key = None;
    let mut kwargs = BTreeMap::new();
    let failure = harness
        .threads()
        .acreate_thread(create_call(&mut kwargs, None))
        .await
        .expect_err("no api key reaches the client");
    assert!(matches!(failure.class, ErrorClass::Authentication(_)));
    assert_sdk_failure(&failure, 401, "AuthenticationError");
}

#[tokio::test]
async fn a_status_less_call_failure_maps_as_api_connection() {
    let harness = Harness::new();
    let mut kwargs = BTreeMap::new();
    let failure = harness
        .threads()
        .aget_thread(GetThreadCall {
            custom_llm_provider: String::from("openai"),
            thread_id: String::new(),
            client: None,
            kwargs: &mut kwargs,
        })
        .await
        .expect_err("an empty thread id fails before the request");
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_sdk_failure(&failure, 500, "APIConnectionError");
    assert_eq!(failure.class.metadata().openai_code, None);
    assert!(harness.recorder.seen().is_empty());
}

#[tokio::test]
async fn a_transport_timeout_maps_as_apitimeout_from_the_async_wrapper() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    let transport = ProxiedClient::build(base_builder().http1_only(), &policy, TIMEOUT_LIMITS)
        .expect("the client");
    let mut harness = Harness::new();
    harness.resources.openai_base_url = Some(format!("http://{address}/v1"));
    let threads = AssistantThreads {
        transport: Arc::new(transport),
        resources: &harness.resources,
        azure: &harness.azure,
        environment: &harness.env,
        sources: &harness.sources,
        redactor: &harness.redactor,
        refresh_enabled: false,
        session: SessionKind::Dedicated,
    };
    let mut kwargs = kwargs_of(r#"{"timeout":0.05}"#);
    let failure = threads
        .acreate_thread(CreateThreadCall {
            custom_llm_provider: String::from("openai"),
            messages: None,
            metadata: None,
            tool_resources: None,
            client: None,
            kwargs: &mut kwargs,
        })
        .await
        .expect_err("the silent listener runs into its deadline");
    assert!(matches!(failure.class, ErrorClass::Timeout(_)));
    assert_sdk_failure(&failure, 408, "APITimeoutError");
    assert_eq!(failure.class.metadata().openai_code, None);
}

#[rstest]
#[case::true_flag("true", "openai:Asynchronous")]
#[case::false_flag("false", "openai:Sync")]
#[case::one("1", "openai:Sync")]
#[case::text(r#""true""#, "openai:Sync")]
#[case::null_flag("null", "openai:Sync")]
#[case::absent("", "openai:Sync")]
#[tokio::test]
async fn only_the_literal_true_flag_selects_the_async_client(
    #[case] flag: &'static str,
    #[case] expected: &'static str,
) {
    let harness = Harness::new();
    let capture = Capture::default();
    let mut kwargs = BTreeMap::new();
    if !flag.is_empty() {
        kwargs.insert(String::from("acreate_thread"), json(flag));
    }
    let thread = execute(
        &harness.threads(),
        FacadeCall::Create(create_call(&mut kwargs, None)),
        &capture,
    )
    .await
    .expect("the call prepares");
    assert!(thread.is_none());
    assert_eq!(capture.calls.lock().unwrap()[0], expected);
}

#[rstest]
#[case::default("{}", 600.0_f64)]
#[case::zero_falls_to_request_timeout(r#"{"timeout":0,"request_timeout":4}"#, 4.0_f64)]
#[case::text(r#"{"timeout":"7.5"}"#, 7.5_f64)]
#[case::empty_text(r#"{"timeout":""}"#, 600.0_f64)]
#[test]
fn timeouts_resolve_like_litellm(#[case] text: &'static str, #[case] expected: f64) {
    let params = ThreadParams::from_kwargs(&kwargs_of(text)).expect("the kwargs parse");
    assert_eq!(
        resolve_timeout(&params),
        Ok(ThreadTimeout::Seconds(expected))
    );
}

#[test]
fn a_negative_timeout_has_no_deadline_and_a_non_finite_one_is_refused() {
    let harness = Harness::new();
    let negative = kwargs_of(r#"{"timeout":-5}"#);
    let prepared = harness
        .threads()
        .prepare(prepare_request(empty_create(), &negative, None))
        .expect("the call prepares");
    let Target::Openai(client) = prepared.target else {
        panic!("openai prepares an openai client");
    };
    assert_eq!(client.timeout, Some(Duration::MAX));
    let infinite = kwargs_of(r#"{"timeout":"inf"}"#);
    let refused = harness
        .threads()
        .prepare(prepare_request(empty_create(), &infinite, None));
    assert!(matches!(
        refused,
        Err(ThreadError::Timeout(TimeoutError::NonFinite))
    ));
}

#[test]
fn a_read_only_structured_timeout_stays_structured() {
    let params = ThreadParams::from_kwargs(&kwargs_of(r#"{"timeout":{"read":3.0,"connect":1.0}}"#))
        .expect("the kwargs parse");
    assert_eq!(
        resolve_timeout(&params),
        Ok(ThreadTimeout::Phases(StructuredTimeout {
            connect: Some(1.0_f64),
            read: Some(3.0_f64),
            write: None,
            pool: None,
        }))
    );
}

#[test]
fn a_read_phase_alone_is_not_absent() {
    let params = ThreadParams::from_kwargs(&kwargs_of(r#"{"timeout":{"read":3.0}}"#))
        .expect("the kwargs parse");
    assert_eq!(
        resolve_timeout(&params),
        Ok(ThreadTimeout::Phases(StructuredTimeout {
            connect: None,
            read: Some(3.0_f64),
            write: None,
            pool: None,
        }))
    );
}

#[test]
fn a_connect_phase_alone_is_not_absent() {
    let params = ThreadParams::from_kwargs(&kwargs_of(r#"{"timeout":{"connect":1.0}}"#))
        .expect("the kwargs parse");
    assert_eq!(
        resolve_timeout(&params),
        Ok(ThreadTimeout::Phases(StructuredTimeout {
            connect: Some(1.0_f64),
            read: None,
            write: None,
            pool: None,
        }))
    );
}

#[test]
fn a_pool_phase_alone_is_not_absent() {
    let params = ThreadParams::from_kwargs(&kwargs_of(r#"{"timeout":{"pool":2.0}}"#))
        .expect("the kwargs parse");
    assert_eq!(
        resolve_timeout(&params),
        Ok(ThreadTimeout::Phases(StructuredTimeout {
            connect: None,
            read: None,
            write: None,
            pool: Some(2.0_f64),
        }))
    );
}

#[test]
fn an_empty_phase_map_falls_back_to_the_default() {
    let params =
        ThreadParams::from_kwargs(&kwargs_of(r#"{"timeout":{}}"#)).expect("the kwargs parse");
    assert_eq!(resolve_timeout(&params), Ok(ThreadTimeout::Seconds(600.0)));
}

#[rstest]
#[case::boolean(r#"{"timeout":true}"#)]
#[case::list(r#"{"timeout":[1]}"#)]
#[case::text_phase(r#"{"timeout":{"read":"3"}}"#)]
#[test]
fn a_wrongly_typed_timeout_is_refused_with_the_field_named(#[case] text: &'static str) {
    let refused = ThreadParams::from_kwargs(&kwargs_of(text));
    let error = refused.expect_err("the timeout type is refused");
    assert_eq!(error.field.as_deref(), Some("timeout"));
}

#[test]
fn a_non_object_extra_body_is_refused_with_the_field_named() {
    let refused = ThreadParams::from_kwargs(&kwargs_of(r#"{"extra_body":"token"}"#));
    let error = refused.expect_err("the extra_body type is refused");
    assert_eq!(error.field.as_deref(), Some("extra_body"));
    assert!(error.message.contains("extra_body"));
}

#[test]
fn openai_credentials_follow_the_shared_chain() {
    let mut harness = Harness::new();
    harness.resources.openai_api_key = Some(SecretString::from("env-key"));
    harness.resources.openai_base_url = Some(String::from("https://env.test/v1"));
    harness.resources.openai_organization = Some(String::from("env-org"));
    let kwargs = kwargs_of(
        r#"{"api_key":"request-key","api_base":"https://request.test/v1","organization":"request-org","max_retries":3}"#,
    );
    let prepared = harness
        .threads()
        .prepare(prepare_request(empty_create(), &kwargs, None))
        .expect("the call prepares");
    let Target::Openai(client) = prepared.target else {
        panic!("openai prepares an openai client");
    };
    let key = client.api_key.as_ref().map(secret_text);
    assert_eq!(key, Some(String::from("request-key")));
    assert_eq!(client.base_url.as_deref(), Some("https://request.test/v1"));
    assert_eq!(client.organization.as_deref(), Some("request-org"));
    assert_eq!(client.max_retries, Some(3));
    assert_eq!(client.timeout, Some(Duration::from_secs(600)));
    let empty = BTreeMap::new();
    let fallback = harness
        .threads()
        .prepare(prepare_request(
            Operation::Retrieve(String::from("thread_a")),
            &empty,
            None,
        ))
        .expect("the call prepares");
    let Target::Openai(fallback_client) = fallback.target else {
        panic!("openai prepares an openai client");
    };
    assert_eq!(
        fallback_client.base_url.as_deref(),
        Some("https://env.test/v1")
    );
    assert_eq!(fallback_client.organization.as_deref(), Some("env-org"));
    assert_eq!(fallback_client.max_retries, None);
}

#[tokio::test]
async fn an_unsupported_provider_is_a_bad_request() {
    let harness = Harness::new();
    let mut kwargs = BTreeMap::new();
    let mut call = create_call(&mut kwargs, None);
    call.custom_llm_provider = String::from("anthropic");
    let error = harness
        .threads()
        .create_thread(call)
        .await
        .expect_err("anthropic is unsupported");
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support anthropic for 'create_thread'. Only 'openai' is supported."
    );
    let failure = error.sdk_failure();
    assert_eq!(failure.class, "BadRequestError");
    assert_eq!(failure.status, 400);
    assert_eq!(failure.kind, None);
    let class = error.error_class("anthropic", harness.threads().redactor);
    let ErrorClass::BadRequest(details) = class else {
        panic!("the unsupported provider is a bad request");
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.model.as_deref(), Some("n/a"));
}

#[tokio::test]
async fn an_unsupported_provider_on_get_names_the_get_facade() {
    let harness = Harness::new();
    let mut kwargs = BTreeMap::new();
    let error = harness
        .threads()
        .get_thread(GetThreadCall {
            custom_llm_provider: String::from("anthropic"),
            thread_id: String::from("thread_a"),
            client: None,
            kwargs: &mut kwargs,
        })
        .await
        .expect_err("anthropic is unsupported");
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support anthropic for 'get_thread'. Only 'openai' is supported."
    );
}

#[tokio::test]
async fn a_provider_404_keeps_the_sdk_fields_synchronously() {
    let harness = Harness::new();
    harness.recorder.queue(404, TYPED_404);
    let mut kwargs = BTreeMap::new();
    let error = harness
        .threads()
        .get_thread(GetThreadCall {
            custom_llm_provider: String::from("openai"),
            thread_id: String::from("thread_a"),
            client: None,
            kwargs: &mut kwargs,
        })
        .await
        .expect_err("the mock failed");
    let failure = error.sdk_failure();
    assert_eq!(failure.class, "NotFoundError");
    assert_eq!(failure.status, 404);
    assert_eq!(failure.kind.as_deref(), Some("invalid_request_error"));
    assert_eq!(failure.param.as_deref(), Some("thread_id"));
    assert_eq!(failure.code.as_deref(), Some("missing"));
}

#[rstest]
#[case::typed(true)]
#[case::untyped(false)]
#[tokio::test]
async fn the_async_wrapper_maps_the_provider_error_through_b011(#[case] typed: bool) {
    let reply = if typed { TYPED_404 } else { UNTYPED_404 };
    let harness = Harness::new();
    harness.recorder.queue(404, reply);
    let mut kwargs = BTreeMap::new();
    let failure = harness
        .threads()
        .aget_thread(GetThreadCall {
            custom_llm_provider: String::from("openai"),
            thread_id: String::from("thread_a"),
            client: None,
            kwargs: &mut kwargs,
        })
        .await
        .expect_err("the mock failed");
    let metadata = failure.class.metadata();
    assert_eq!(metadata.status_code, 404);
    assert_eq!(metadata.provider.as_deref(), Some("openai"));
    if typed {
        assert!(matches!(failure.class, ErrorClass::BadRequest(_)));
        assert_eq!(
            metadata.error_type,
            WireField::Text(String::from("invalid_request_error"))
        );
        assert_eq!(metadata.param, WireField::Text(String::from("thread_id")));
        assert_eq!(metadata.openai_code.as_deref(), Some("missing"));
    } else {
        assert!(matches!(failure.class, ErrorClass::NotFound(_)));
        assert_eq!(metadata.error_type, WireField::Null);
        assert_eq!(metadata.param, WireField::Null);
        assert_eq!(metadata.openai_code, None);
    }
}

#[test]
fn the_operation_names_match_the_litellm_message() {
    assert_eq!(ThreadOperation::Create.name(), "create_thread");
    assert_eq!(ThreadOperation::Get.name(), "get_thread");
    let created = create_body(None, None).expect("empty metadata converts");
    assert_eq!(
        created,
        ThreadCreate {
            messages: None,
            metadata: None,
        }
    );
}

#[rstest]
#[case::bad_request(400, "BadRequestError")]
#[case::authentication(401, "AuthenticationError")]
#[case::permission(403, "PermissionDeniedError")]
#[case::not_found(404, "NotFoundError")]
#[case::conflict(409, "ConflictError")]
#[case::unprocessable(422, "UnprocessableEntityError")]
#[case::rate_limit(429, "RateLimitError")]
#[case::server_error(500, "InternalServerError")]
#[case::service_unavailable(503, "InternalServerError")]
fn sdk_class_names_every_status_like_the_pinned_sdk(
    #[case] status: u16,
    #[case] class: &'static str,
) {
    assert_eq!(sdk_class(status), class);
}

#[test]
fn forwarded_azure_timeout_reads_numbers() {
    let mut params = NO_PARAMS;
    params.timeout = Some(ThreadTimeoutValue::Seconds(30.0_f64));
    assert_eq!(forwarded_azure_timeout(&params), Ok(Some(30.0_f64)));
    params.timeout = Some(ThreadTimeoutValue::Seconds(f64::INFINITY));
    assert_eq!(
        forwarded_azure_timeout(&params),
        Err(TimeoutError::NonFinite)
    );
    params.timeout = Some(ThreadTimeoutValue::Seconds(f64::NEG_INFINITY));
    assert_eq!(
        forwarded_azure_timeout(&params),
        Err(TimeoutError::NonFinite)
    );
    params.timeout = Some(ThreadTimeoutValue::Seconds(-5.0));
    assert_eq!(forwarded_azure_timeout(&params), Ok(None));
    params.timeout = Some(ThreadTimeoutValue::Seconds(0.0));
    assert_eq!(forwarded_azure_timeout(&params), Ok(None));
    params.timeout = None;
    assert_eq!(forwarded_azure_timeout(&params), Ok(None));
}

#[test]
fn forwarded_azure_timeout_reads_phases() {
    let mut params = NO_PARAMS;
    params.timeout = Some(ThreadTimeoutValue::Phases(StructuredTimeout {
        connect: None,
        read: Some(3.0_f64),
        write: None,
        pool: None,
    }));
    assert_eq!(forwarded_azure_timeout(&params), Ok(Some(3.0_f64)));
    params.timeout = Some(ThreadTimeoutValue::Phases(StructuredTimeout {
        connect: None,
        read: Some(0.0_f64),
        write: None,
        pool: None,
    }));
    assert_eq!(forwarded_azure_timeout(&params), Ok(None));
    params.timeout = Some(ThreadTimeoutValue::Phases(StructuredTimeout {
        connect: Some(1.0_f64),
        read: None,
        write: None,
        pool: None,
    }));
    assert_eq!(forwarded_azure_timeout(&params), Ok(None));
}

#[rstest]
#[case::plain("7.5", 7.5_f64, true)]
#[case::underscores("7_5", 75.0_f64, true)]
#[case::underscore_after_sign("-_1", 0.0_f64, false)]
#[case::trailing_underscore("75_", 0.0_f64, false)]
#[case::leading_underscore("_75", 0.0_f64, false)]
#[case::double_underscore("1__2", 0.0_f64, false)]
fn forwarded_azure_timeout_reads_text_like_python_float(
    #[case] text: &'static str,
    #[case] expected: f64,
    #[case] valid: bool,
) {
    let mut params = NO_PARAMS;
    params.timeout = Some(ThreadTimeoutValue::Text(String::from(text)));
    let result = forwarded_azure_timeout(&params);
    if valid {
        assert_eq!(result, Ok(Some(expected)));
    } else {
        assert!(matches!(result, Err(TimeoutError::InvalidNumber { .. })));
    }
}

#[rstest]
#[case::negative_number(r#"{"timeout":-5}"#)]
#[case::zero_number(r#"{"timeout":0}"#)]
#[case::negative_text(r#"{"timeout":"-5"}"#)]
#[case::zero_text(r#"{"timeout":"0"}"#)]
#[case::empty_text(r#"{"timeout":""}"#)]
#[case::negative_read_phase(r#"{"timeout":{"read":-1}}"#)]
#[test]
fn a_non_positive_azure_timeout_leaves_the_client_default(#[case] text: &'static str) {
    let params = ThreadParams::from_kwargs(&kwargs_of(text)).expect("the kwargs parse");
    assert_eq!(forwarded_azure_timeout(&params), Ok(None));
}

mod azure;

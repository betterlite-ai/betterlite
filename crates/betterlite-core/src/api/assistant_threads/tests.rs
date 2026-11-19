use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_config::sections::azure_endpoints;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::assistants::client::{ClientKind, SdkClient};
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::records::{
    MessageContent, MessageRole, ThreadCreate, ThreadMessageData, ThreadRecord,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::ThreadError;
use super::params::{StructuredTimeout, ThreadTimeout};
use super::runtime::ThreadRuntime;
use super::target::{Operation, PrepareRequest, PreparedThread, Target, create_body};
use super::{AssistantThreads, CreateThreadCall, GetThreadCall, SuppliedClient, ThreadOperation};
use crate::api::resource_sdk_settings::ResourceSdkSettings;

const THREAD: &str = r#"{"id":"thread_a","object":"thread","created_at":1,"metadata":{"tag":"x"},"tool_resources":{}}"#;
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
    timeouts: Mutex<Vec<ThreadTimeout>>,
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
        self.timeouts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(prepared.timeout);
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
            session: betterlite_azure::endpoints::projection::SessionKind::Dedicated,
        }
    }
}

fn empty_resources() -> ResourceSdkSettings {
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

fn create_call<'call>(
    kwargs: &'call mut BTreeMap<String, Json>,
    messages: Option<Vec<ThreadMessageData>>,
) -> CreateThreadCall<'call, Recorder> {
    CreateThreadCall {
        custom_llm_provider: String::from("openai"),
        messages,
        metadata: Some([("tag".to_owned(), json(r#""x""#))].into_iter().collect()),
        tool_resources: Some(json(r#"{"file_search":{"vector_store_ids":["vs_a"]}}"#)),
        client: None,
        kwargs,
    }
}

fn get_call<'call>(kwargs: &'call mut BTreeMap<String, Json>) -> GetThreadCall<'call, Recorder> {
    GetThreadCall {
        custom_llm_provider: String::from("openai"),
        thread_id: String::from("thread_a"),
        client: None,
        kwargs,
    }
}

fn prepare_request<'call>(
    operation: Operation,
    kwargs: &'call BTreeMap<String, Json>,
    flag: Option<bool>,
) -> PrepareRequest<'call, Recorder> {
    PrepareRequest {
        provider: String::from("openai"),
        operation,
        client: None,
        kwargs,
        facade: ThreadOperation::Create,
        flag,
    }
}

fn empty_create() -> Operation {
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
    let _ = harness.threads().create_thread(call).await.expect("create");
    assert_eq!(harness.recorder.seen()[0].body, json("{}"));
    let mut kwargs = BTreeMap::new();
    let mut call = create_call(&mut kwargs, Some(Vec::new()));
    call.metadata = Some(BTreeMap::new());
    let _ = harness.threads().create_thread(call).await.expect("create");
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
        .get_thread(get_call(&mut kwargs))
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
    let thread = harness
        .threads()
        .create_thread_with(create_call(&mut kwargs, None), &capture)
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
    let mut kwargs = BTreeMap::new();
    kwargs.insert(String::from("aget_thread"), Json::Bool(true));
    let _ = harness
        .threads()
        .aget_thread(get_call(&mut kwargs))
        .await
        .expect("the retrieve succeeds");
    assert!(!kwargs.contains_key("aget_thread"));
}

#[tokio::test]
async fn only_the_literal_true_flag_selects_the_async_client() {
    for (flag, expected) in [
        (Json::Bool(true), "openai:Async"),
        (Json::Bool(false), "openai:Sync"),
        (json("1"), "openai:Sync"),
        (json(r#""true""#), "openai:Sync"),
        (Json::Null, "openai:Sync"),
    ] {
        let harness = Harness::new();
        let capture = Capture::default();
        let mut kwargs = BTreeMap::new();
        kwargs.insert(String::from("acreate_thread"), flag);
        let _ = harness
            .threads()
            .create_thread_with(create_call(&mut kwargs, None), &capture)
            .await
            .expect("the call prepares");
        assert_eq!(capture.calls.lock().unwrap()[0], expected);
    }
    let harness = Harness::new();
    let capture = Capture::default();
    let mut kwargs = BTreeMap::new();
    let _ = harness
        .threads()
        .create_thread_with(create_call(&mut kwargs, None), &capture)
        .await
        .expect("the call prepares");
    assert_eq!(capture.calls.lock().unwrap()[0], "openai:Sync");
}

#[tokio::test]
async fn timeouts_resolve_like_litellm() {
    for (text, expected) in [
        ("{}", ThreadTimeout::Seconds(600.0)),
        (
            r#"{"timeout":0,"request_timeout":4}"#,
            ThreadTimeout::Seconds(4.0),
        ),
        (r#"{"timeout":"7.5"}"#, ThreadTimeout::Seconds(7.5)),
        (
            r#"{"timeout":{"read":3.0,"connect":1.0}}"#,
            ThreadTimeout::Phases(StructuredTimeout {
                connect: Some(1.0),
                read: Some(3.0),
                write: None,
                pool: None,
            }),
        ),
    ] {
        let harness = Harness::new();
        let capture = Capture::default();
        let mut kwargs = kwargs_of(text);
        let _ = harness
            .threads()
            .create_thread_with(create_call(&mut kwargs, None), &capture)
            .await
            .expect("the call prepares");
        assert_eq!(capture.timeouts.lock().unwrap()[0], expected);
    }
}

#[tokio::test]
async fn openai_credentials_follow_the_shared_chain() {
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
    assert_eq!(client.kind, ClientKind::Sync);
    assert_eq!(
        client
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("request-key")
    );
    assert_eq!(client.base_url.as_deref(), Some("https://request.test/v1"));
    assert_eq!(client.organization.as_deref(), Some("request-org"));
    assert_eq!(client.max_retries, Some(3));
    assert_eq!(client.timeout, Some(std::time::Duration::from_secs(600)));
    let empty = BTreeMap::new();
    let prepared = harness
        .threads()
        .prepare(prepare_request(
            Operation::Retrieve(String::from("thread_a")),
            &empty,
            None,
        ))
        .expect("the call prepares");
    let Target::Openai(client) = prepared.target else {
        panic!("openai prepares an openai client");
    };
    assert_eq!(client.base_url.as_deref(), Some("https://env.test/v1"));
    assert_eq!(client.organization.as_deref(), Some("env-org"));
    assert_eq!(client.max_retries, None);
}

#[tokio::test]
async fn azure_gets_litellm_params_and_the_shared_settings() {
    let mut harness = Harness::new();
    harness.resources.azure_api_base = Some(String::from("https://azure.test"));
    harness.resources.azure_api_version = Some(String::from("2024-02-01"));
    harness.resources.azure_openai_api_key = Some(SecretString::from("azure-key"));
    let kwargs = kwargs_of(
        r#"{"tenant_id":"tenant","client_secret":"secret","max_retries":5,"timeout":"7.5","azure_ad_token":"param-token"}"#,
    );
    let prepared = harness
        .threads()
        .prepare(PrepareRequest {
            provider: String::from("azure"),
            operation: Operation::Retrieve(String::from("thread_a")),
            client: None,
            kwargs: &kwargs,
            facade: ThreadOperation::Get,
            flag: None,
        })
        .expect("the call prepares");
    let Target::Azure {
        request, client, ..
    } = prepared.target
    else {
        panic!("azure prepares an azure client");
    };
    assert_eq!(request.api_base.as_deref(), Some("https://azure.test"));
    assert_eq!(request.api_version.as_deref(), Some("2024-02-01"));
    assert_eq!(
        request
            .params
            .azure_ad_token
            .as_ref()
            .map(|token| token.expose_secret().as_str()),
        Some("param-token")
    );
    assert_eq!(request.params.auth.tenant_id.as_deref(), Some("tenant"));
    assert_eq!(request.params.max_retries, Some(5));
    assert_eq!(request.params.timeout, Some(7.5));
    assert_eq!(client.kind, ClientKind::Sync);
    assert_eq!(client.api_version, "2024-02-01");
    assert_eq!(client.deadline, std::time::Duration::from_secs_f64(7.5));
}

#[tokio::test]
async fn azure_token_sources_are_absent_null_or_body() {
    let mut harness = Harness::new();
    harness.resources.azure_ad_token = Some(SecretString::from("env-token"));
    for (extra, expected) in [
        (None, None),
        (Some(Json::Null), Some("env-token")),
        (
            Some(json(r#"{"azure_ad_token":"body-token"}"#)),
            Some("body-token"),
        ),
    ] {
        let mut kwargs = BTreeMap::new();
        if let Some(value) = extra {
            kwargs.insert(String::from("extra_body"), value);
        }
        let prepared = harness
            .threads()
            .prepare(PrepareRequest {
                provider: String::from("azure"),
                operation: Operation::Retrieve(String::from("thread_a")),
                client: None,
                kwargs: &kwargs,
                facade: ThreadOperation::Get,
                flag: None,
            })
            .expect("the call prepares");
        let Target::Azure { request, .. } = prepared.target else {
            panic!("azure prepares an azure client");
        };
        assert_eq!(
            request
                .azure_ad_token
                .as_ref()
                .map(|token| token.expose_secret().as_str()),
            expected
        );
    }
}

fn sdk_client(kind: ClientKind, transport: Arc<Recorder>) -> SdkClient<Recorder> {
    SdkClient {
        kind,
        api_key: Some(SecretString::from("supplied-key")),
        base_url: Some(String::from("https://supplied.test/v1")),
        timeout: Some(std::time::Duration::from_secs(42)),
        max_retries: Some(0),
        organization: None,
        transport,
    }
}

fn azure_client(
    kind: ClientKind,
    transport: Arc<Recorder>,
) -> AzureAssistantsClient<'static, Recorder> {
    AzureAssistantsClient {
        kind,
        inputs: None,
        transport,
        base_url: String::from("https://supplied-azure.test"),
        api_version: String::from("2024-01-01"),
        key: Some(SecretString::from("supplied-azure-key")),
        token: None,
        deadline: std::time::Duration::from_secs(42),
    }
}

#[tokio::test]
async fn azure_drops_the_sync_sdk_classes_only() {
    let harness = Harness::new();
    for (supplied, expected) in [
        (
            SuppliedClient::Openai(sdk_client(ClientKind::Sync, Arc::default())),
            "azure:Sync:None",
        ),
        (
            SuppliedClient::Openai(sdk_client(ClientKind::Async, Arc::default())),
            "openai:Async",
        ),
        (
            SuppliedClient::Azure(azure_client(ClientKind::Sync, Arc::default())),
            "azure:Sync:None",
        ),
        (
            SuppliedClient::Azure(azure_client(ClientKind::Async, Arc::default())),
            "azure:Async:None",
        ),
    ] {
        let capture = Capture::default();
        let mut kwargs = BTreeMap::new();
        let mut call = create_call(&mut kwargs, None);
        call.custom_llm_provider = String::from("azure");
        call.client = Some(supplied);
        let _ = harness
            .threads()
            .create_thread_with(call, &capture)
            .await
            .expect("the call prepares");
        assert_eq!(capture.calls.lock().unwrap()[0], expected);
    }
}

#[tokio::test]
async fn azure_create_posts_to_threads_with_api_key_auth() {
    let mut harness = Harness::new();
    harness.resources.azure_api_base = Some(String::from("https://azure.test"));
    harness.resources.azure_api_version = Some(String::from("2024-02-01"));
    harness.resources.azure_openai_api_key = Some(SecretString::from("azure-key"));
    harness.recorder.queue(200, THREAD);
    let mut kwargs = BTreeMap::new();
    let mut call = create_call(&mut kwargs, Some(vec![message("hello")]));
    call.custom_llm_provider = String::from("azure");
    let thread = harness
        .threads()
        .create_thread(call)
        .await
        .expect("the create succeeds")
        .expect("the thread returns");
    assert_eq!(thread.id, "thread_a");
    let seen = &harness.recorder.seen()[0];
    assert_eq!(seen.method, "POST");
    assert_eq!(
        seen.url,
        "https://azure.test/openai/threads?api-version=2024-02-01"
    );
    assert_eq!(seen.api_key.as_deref(), Some("azure-key"));
    assert_eq!(seen.beta.as_deref(), Some("assistants=v2"));
    assert_eq!(
        seen.body,
        json(r#"{"messages":[{"role":"user","content":"hello"}],"metadata":{"tag":"x"}}"#)
    );
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
async fn a_provider_404_keeps_the_sdk_fields_synchronously() {
    let harness = Harness::new();
    harness.recorder.queue(404, TYPED_404);
    let mut kwargs = BTreeMap::new();
    let error = harness
        .threads()
        .get_thread(get_call(&mut kwargs))
        .await
        .expect_err("the mock failed");
    let failure = error.sdk_failure();
    assert_eq!(failure.class, "NotFoundError");
    assert_eq!(failure.status, 404);
    assert_eq!(failure.kind.as_deref(), Some("invalid_request_error"));
    assert_eq!(failure.param.as_deref(), Some("thread_id"));
    assert_eq!(failure.code.as_deref(), Some("missing"));
}

#[tokio::test]
async fn the_async_wrapper_maps_the_provider_error_through_b011() {
    for (reply, typed) in [(TYPED_404, true), (UNTYPED_404, false)] {
        let harness = Harness::new();
        harness.recorder.queue(404, reply);
        let mut kwargs = BTreeMap::new();
        let failure = harness
            .threads()
            .aget_thread(get_call(&mut kwargs))
            .await
            .expect_err("the mock failed");
        let metadata = failure.class.metadata();
        assert_eq!(metadata.status_code, 404);
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
}

#[tokio::test]
async fn an_azure_provider_404_maps_without_body_fields_asynchronously() {
    let mut harness = Harness::new();
    harness.resources.azure_api_base = Some(String::from("https://azure.test"));
    harness.resources.azure_openai_api_key = Some(SecretString::from("azure-key"));
    harness.recorder.queue(404, TYPED_404);
    let mut kwargs = BTreeMap::new();
    let mut call = get_call(&mut kwargs);
    call.custom_llm_provider = String::from("azure");
    let failure = harness
        .threads()
        .aget_thread(call)
        .await
        .expect_err("the mock failed");
    let metadata = failure.class.metadata();
    assert!(matches!(failure.class, ErrorClass::NotFound(_)));
    assert_eq!(metadata.status_code, 404);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
    assert_eq!(harness.recorder.seen().len(), 1);
}

#[test]
fn the_operation_names_match_the_litellm_message() {
    assert_eq!(ThreadOperation::Create.to_string(), "create_thread");
    assert_eq!(ThreadOperation::Get.to_string(), "get_thread");
    let created = create_body(None, None).expect("empty metadata converts");
    assert_eq!(
        created,
        ThreadCreate {
            messages: None,
            metadata: None,
        }
    );
}

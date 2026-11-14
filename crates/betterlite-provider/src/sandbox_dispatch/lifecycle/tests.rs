#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::Method;
use http::header::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use url::Url;

use crate::e2b::{
    E2bError, E2bSandbox, SandboxHttp as E2bHttp, SandboxRequest as E2bRequest,
    SandboxStream as E2bStream,
};
use crate::opensandbox::helpers::{
    OpenSandbox, OpenSandboxError, PollClock, SandboxHttp as OpenHttp,
    SandboxRequest as OpenRequest, SandboxStream as OpenStream,
};
use crate::sandbox_dispatch::error::SandboxLifecycleError;
use crate::sandbox_dispatch::logging::{SandboxLogUpdate, SandboxLogging};
use crate::sandbox_dispatch::request::{
    CreateSandbox, DeleteSandbox, RunCode, SandboxCall, SandboxContainer, SandboxContainerRef,
};

use super::SandboxDispatch;

type Environment = BTreeMap<String, String>;

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Vec<u8>,
}

#[derive(Clone)]
struct FakeHttp {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Recorded {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl FakeHttp {
    fn new(replies: &[(u16, &str)]) -> Self {
        Self {
            replies: Arc::new(Mutex::new(
                replies
                    .iter()
                    .map(|(status, body)| Reply {
                        status: *status,
                        body: body.as_bytes().to_vec(),
                    })
                    .collect(),
            )),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn only_request(&self) -> Recorded {
        let mut requests = self.requests();
        assert_eq!(recorded.len(), 1_usize);
        requests.remove(0)
    }

    fn next_reply(&self) -> Reply {
        let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        match replies.len() {
            0 => Reply {
                status: 200,
                body: Vec::new(),
            },
            1 => replies.front().cloned().unwrap(),
            _ => replies.pop_front().unwrap(),
        }
    }

    fn record(&self, method: &Method, url: &Url, headers: &HeaderMap, body: Vec<u8>) {
        let headers = headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value
                        .to_str()
                        .expect("the fixture headers are visible ASCII")
                        .to_owned(),
                )
            })
            .collect();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                method: method.as_str().to_owned(),
                url: url.to_string(),
                headers,
                body,
            });
    }
}

#[async_trait]
impl E2bHttp for FakeHttp {
    async fn send(&self, request: E2bRequest) -> Result<E2bStream, E2bError> {
        self.record(
            &request.method,
            &request.url,
            &request.headers,
            request.body,
        );
        let reply = self.next_reply();
        Ok(E2bStream {
            status: reply.status,
            body: stream::iter(vec![Ok::<Vec<u8>, E2bError>(reply.body)]).boxed(),
        })
    }
}

#[async_trait]
impl OpenHttp for FakeHttp {
    async fn send(&self, request: OpenRequest) -> Result<OpenStream, OpenSandboxError> {
        self.record(
            &request.method,
            &request.url,
            &request.headers,
            request.body,
        );
        let reply = self.next_reply();
        Ok(OpenStream {
            status: reply.status,
            body: stream::iter(vec![Ok::<Vec<u8>, OpenSandboxError>(reply.body)]).boxed(),
        })
    }
}

#[derive(Clone, Default)]
struct StepClock {
    now_millis: Arc<AtomicU64>,
}

#[async_trait]
impl PollClock for StepClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.now_millis.load(Ordering::Relaxed))
    }

    async fn sleep(&self, duration: Duration) {
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        self.now_millis.fetch_add(millis, Ordering::Relaxed);
    }
}

type Dispatch = SandboxDispatch<
    E2bSandbox<FakeHttp, Environment>,
    OpenSandbox<FakeHttp, Environment, StepClock>,
>;

fn build_dispatch(http: &FakeHttp, environment: Environment) -> Dispatch {
    SandboxDispatch {
        e2b: E2bSandbox {
            http: http.clone(),
            environment: environment.clone(),
        },
        opensandbox: OpenSandbox {
            http: http.clone(),
            environment,
            clock: StepClock::default(),
        },
    }
}

fn call<'call>(
    provider: &'call str,
    api_key: Option<&'call SecretString>,
    kwargs: &'call IndexMap<String, Json>,
) -> SandboxCall<'call> {
    SandboxCall {
        provider,
        api_key,
        kwargs,
        logging: None,
    }
}

fn create_request<'call>(
    provider: &'call str,
    api_key: Option<&'call SecretString>,
    kwargs: &'call IndexMap<String, Json>,
) -> CreateSandbox<'call> {
    CreateSandbox {
        call: call(provider, api_key, kwargs),
        api_base: None,
        template: None,
        timeout: None,
        allow_internet_access: None,
        metadata: None,
        env_vars: None,
        resource_limits: None,
        resource_requests: None,
        entrypoint: None,
        network_policy: None,
        secure_access: false,
        use_server_proxy: false,
        ready_timeout: None,
        poll_interval: None,
    }
}

fn run_request<'call>(
    provider: &'call str,
    container: SandboxContainerRef<'call>,
    code: &'call str,
    kwargs: &'call IndexMap<String, Json>,
) -> RunCode<'call> {
    RunCode {
        call: call(provider, None, kwargs),
        container,
        code,
        api_base: None,
        env_vars: None,
        language: None,
        use_server_proxy: false,
        ready_timeout: None,
        poll_interval: None,
    }
}

fn delete_request<'call>(
    provider: &'call str,
    container: SandboxContainerRef<'call>,
    kwargs: &'call IndexMap<String, Json>,
) -> DeleteSandbox<'call> {
    DeleteSandbox {
        call: call(provider, None, kwargs),
        container,
        api_base: None,
    }
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

fn header<'recorded>(recorded: &'recorded Recorded, name: &str) -> &'recorded str {
    recorded
        .headers
        .get(name)
        .map_or_else(|| panic!("the request carries {name}"), String::as_str)
}

fn body(recorded: &Recorded) -> String {
    String::from_utf8(recorded.body.clone()).expect("the fixture body is UTF-8")
}

fn environment(pairs: &[(&str, &str)]) -> Environment {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn empty_kwargs() -> IndexMap<String, Json> {
    IndexMap::new()
}

const E2B_CREATED: (u16, &str) = (
    200,
    r#"{"sandboxID":"sbx_1","envdAccessToken":"envd-token"}"#,
);
const EXECUTED: (u16, &str) = (200, "{\"type\":\"stdout\",\"text\":\"hello\"}");

#[tokio::test]
async fn acreate_sandbox_creates_with_the_e2b_defaults() {
    let http = FakeHttp::new(&[E2B_CREATED]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("e2b-key");
    let kwargs = empty_kwargs();
    let mut request = create_request("e2b", Some(&api_key), &kwargs);
    let created = sandbox.acreate_sandbox(&mut request).await.unwrap();
    let SandboxContainer::E2b(handle) = created else {
        panic!("e2b returns an e2b container");
    };
    assert_eq!(handle.id, "sbx_1");
    assert_eq!(handle.provider, "e2b");
    let sent = http.only_request();
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.url, "https://api.e2b.app/sandboxes");
    assert_eq!(header(&sent, "x-api-key"), "e2b-key");
    assert_eq!(
        body(&sent),
        r#"{"templateID":"code-interpreter-v1","timeout":300,"secure":true,"allow_internet_access":true}"#
    );
}

#[tokio::test]
async fn acreate_sandbox_forwards_the_e2b_overrides() {
    let http = FakeHttp::new(&[E2B_CREATED]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("override-key");
    let kwargs = empty_kwargs();
    let mut request = create_request("e2b", Some(&api_key), &kwargs);
    request.api_base = Some("https://e2b.test");
    request.template = Some("my-template");
    request.timeout = Some(60);
    request.allow_internet_access = Some(false);
    sandbox.acreate_sandbox(&mut request).await.unwrap();
    let sent = http.only_request();
    assert_eq!(sent.url, "https://e2b.test/sandboxes");
    assert_eq!(
        body(&sent),
        r#"{"templateID":"my-template","timeout":60,"secure":true,"allow_internet_access":false}"#
    );
}

#[tokio::test]
async fn acreate_sandbox_forwards_the_opensandbox_options() {
    let http = FakeHttp::new(&[
        (200, r#"{"id":"sbx_9","status":{"state":"Running"}}"#),
        (
            200,
            r#"{"endpoint":"http://127.0.0.1:44772","headers":{"x-execd":"tok"}}"#,
        ),
    ]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("open-key");
    let kwargs = empty_kwargs();
    let metadata = BTreeMap::from([("team".to_owned(), "a".to_owned())]);
    let env_vars = BTreeMap::from([("MODE".to_owned(), "test".to_owned())]);
    let resource_requests = BTreeMap::from([("cpu".to_owned(), "2".to_owned())]);
    let entrypoint = ["/bin/run".to_owned()];
    let network_policy =
        serde_json::from_str::<Box<serde_json::value::RawValue>>(r#"{"defaultAction":"allow"}"#)
            .unwrap();
    let mut request = create_request("opensandbox", Some(&api_key), &kwargs);
    request.api_base = Some("https://open.test");
    request.template = Some("template:v1");
    request.timeout = Some(90);
    request.metadata = Some(&metadata);
    request.env_vars = Some(&env_vars);
    request.resource_requests = Some(&resource_requests);
    request.entrypoint = Some(&entrypoint);
    request.network_policy = Some(&network_policy);
    request.secure_access = true;
    let created = sandbox.acreate_sandbox(&mut request).await.unwrap();
    let SandboxContainer::OpenSandbox(handle) = created else {
        panic!("opensandbox returns an opensandbox container");
    };
    assert_eq!(handle.container.id, "sbx_9");
    let recorded = http.requests();
    assert_eq!(recorded[0].method, "POST");
    assert_eq!(recorded[0].url, "https://open.test/sandboxes");
    assert_eq!(header(&recorded[0], "open-sandbox-api-key"), "open-key");
    assert_eq!(
        body(&recorded[0]),
        r#"{"image":{"uri":"template:v1"},"entrypoint":["/bin/run"],"timeout":90,"resourceLimits":{"cpu":"1","memory":"2Gi"},"metadata":{"team":"a"},"env":{"MODE":"test"},"resourceRequests":{"cpu":"2"},"networkPolicy":{"defaultAction":"allow"},"secureAccess":true}"#
    );
}

#[tokio::test]
async fn acreate_sandbox_forwards_the_server_proxy_flag() {
    let http = FakeHttp::new(&[
        (200, r#"{"id":"sbx_9","status":{"state":"Running"}}"#),
        (200, r#"{"endpoint":"http://127.0.0.1:44772","headers":{}}"#),
    ]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("open-key");
    let kwargs = empty_kwargs();
    let mut request = create_request("opensandbox", Some(&api_key), &kwargs);
    request.api_base = Some("https://open.test");
    request.use_server_proxy = true;
    sandbox.acreate_sandbox(&mut request).await.unwrap();
    let recorded = http.requests();
    assert_eq!(
        recorded[1].url,
        "https://open.test/sandboxes/sbx_9/endpoints/44772?use_server_proxy=true"
    );
}

#[tokio::test]
async fn create_reads_the_opensandbox_key_from_the_environment() {
    let http = FakeHttp::new(&[
        (200, r#"{"id":"sbx_9","status":{"state":"Running"}}"#),
        (200, r#"{"endpoint":"http://127.0.0.1:44772"}"#),
    ]);
    let sandbox = build_dispatch(&http, environment(&[("OPEN_SANDBOX_API_KEY", "env-key")]));
    let kwargs = empty_kwargs();
    let mut request = create_request("opensandbox", None, &kwargs);
    request.api_base = Some("https://open.test");
    sandbox.acreate_sandbox(&mut request).await.unwrap();
    let recorded = http.requests();
    assert_eq!(header(&recorded[0], "open-sandbox-api-key"), "env-key");
}

#[tokio::test]
async fn create_with_an_unsupported_provider_names_it() {
    let http = FakeHttp::new(&[]);
    let sandbox = build_dispatch(&http, Environment::new());
    let kwargs = empty_kwargs();
    let mut request = create_request("bedrock", None, &kwargs);
    let error = sandbox.acreate_sandbox(&mut request).await.unwrap_err();
    assert!(matches!(
        error,
        SandboxLifecycleError::UnsupportedProvider { ref provider } if provider == "bedrock"
    ));
    assert!(error.to_string().contains("bedrock"));
}

#[tokio::test]
async fn run_with_an_unsupported_provider_names_it() {
    let http = FakeHttp::new(&[]);
    let sandbox = build_dispatch(&http, Environment::new());
    let kwargs = empty_kwargs();
    let mut request = run_request("bedrock", SandboxContainerRef::Id("sbx_1"), "1", &kwargs);
    let error = sandbox.arun_code(&mut request).await.unwrap_err();
    assert!(matches!(
        error,
        SandboxLifecycleError::UnsupportedProvider { ref provider } if provider == "bedrock"
    ));
}

#[tokio::test]
async fn delete_with_an_unsupported_provider_names_it() {
    let http = FakeHttp::new(&[]);
    let sandbox = build_dispatch(&http, Environment::new());
    let kwargs = empty_kwargs();
    let mut request = delete_request("bedrock", SandboxContainerRef::Id("sbx_1"), &kwargs);
    let error = sandbox.adelete_sandbox(&mut request).await.unwrap_err();
    assert!(matches!(
        error,
        SandboxLifecycleError::UnsupportedProvider { ref provider } if provider == "bedrock"
    ));
}

#[derive(Default)]
struct RecordingLogging {
    updates: Vec<LoggedUpdate>,
}

struct LoggedUpdate {
    model: String,
    provider: String,
    litellm_params: IndexMap<String, Json>,
    kwargs: IndexMap<String, Json>,
}

impl SandboxLogging for RecordingLogging {
    fn update(&mut self, update: SandboxLogUpdate<'_>) -> Result<(), LoggingStateError> {
        self.updates.push(LoggedUpdate {
            model: update.model,
            provider: update.custom_llm_provider,
            litellm_params: update.litellm_params,
            kwargs: update.kwargs.clone(),
        });
        Ok(())
    }
}

#[tokio::test]
async fn create_logs_the_operation_model_with_the_provider() {
    let http = FakeHttp::new(&[E2B_CREATED]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("e2b-key");
    let mut kwargs = empty_kwargs();
    kwargs.insert(
        "litellm_call_id".to_owned(),
        Json::Text("call-7".to_owned()),
    );
    let mut logging = RecordingLogging::default();
    let mut request = create_request("e2b", Some(&api_key), &kwargs);
    request.call.logging = Some(&mut logging);
    sandbox.acreate_sandbox(&mut request).await.unwrap();
    assert_eq!(logging.updates.len(), 1_usize);
    let update = &logging.updates[0];
    assert_eq!(update.model, "e2b/create_sandbox");
    assert_eq!(update.provider, "e2b");
    assert_eq!(
        update.litellm_params.get("litellm_call_id"),
        Some(&Json::Text("call-7".to_owned()))
    );
    assert_eq!(
        update.kwargs.get("litellm_call_id"),
        Some(&Json::Text("call-7".to_owned()))
    );
}

#[tokio::test]
async fn run_logs_the_run_code_operation() {
    let http = FakeHttp::new(&[E2B_CREATED, EXECUTED]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("e2b-key");
    let kwargs = empty_kwargs();
    let mut logging = RecordingLogging::default();
    let mut create_call = create_request("e2b", Some(&api_key), &kwargs);
    create_call.call.logging = Some(&mut logging);
    let created = sandbox.acreate_sandbox(&mut create_call).await.unwrap();
    let mut run = run_request(
        "e2b",
        SandboxContainerRef::Container(&created),
        "print(1)",
        &kwargs,
    );
    sandbox.arun_code(&mut run).await.unwrap();
    assert_eq!(logging.updates.len(), 1_usize);
    assert_eq!(logging.updates[0].model, "e2b/run_code");
}

#[tokio::test]
async fn logging_runs_before_an_unsupported_provider_is_reported() {
    let http = FakeHttp::new(&[]);
    let sandbox = build_dispatch(&http, Environment::new());
    let kwargs = empty_kwargs();
    let mut logging = RecordingLogging::default();
    let mut request = create_request("bedrock", None, &kwargs);
    request.call.logging = Some(&mut logging);
    assert!(sandbox.acreate_sandbox(&mut request).await.is_err());
    assert_eq!(logging.updates.len(), 1_usize);
    assert_eq!(logging.updates[0].model, "bedrock/create_sandbox");
}

#[tokio::test]
async fn internal_kwargs_do_not_reach_the_create_request() {
    let with_kwargs = internal_kwargs();
    let http = FakeHttp::new(&[E2B_CREATED, E2B_CREATED]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("e2b-key");
    let kwargs = empty_kwargs();
    let mut plain = create_request("e2b", Some(&api_key), &kwargs);
    sandbox.acreate_sandbox(&mut plain).await.unwrap();
    let mut internal = create_request("e2b", Some(&api_key), &with_kwargs);
    sandbox.acreate_sandbox(&mut internal).await.unwrap();
    let recorded = http.requests();
    assert_eq!(recorded.len(), 2_usize);
    assert_eq!(recorded[0], recorded[1]);
}

#[tokio::test]
async fn internal_kwargs_do_not_reach_the_run_request() {
    let with_kwargs = internal_kwargs();
    let http = FakeHttp::new(&[E2B_CREATED, EXECUTED, E2B_CREATED, EXECUTED]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("e2b-key");
    let kwargs = empty_kwargs();
    let mut first = create_request("e2b", Some(&api_key), &kwargs);
    let created = sandbox.acreate_sandbox(&mut first).await.unwrap();
    let mut plain = run_request(
        "e2b",
        SandboxContainerRef::Container(&created),
        "1",
        &kwargs,
    );
    sandbox.arun_code(&mut plain).await.unwrap();
    let mut second = create_request("e2b", Some(&api_key), &kwargs);
    let reused = sandbox.acreate_sandbox(&mut second).await.unwrap();
    let mut internal = run_request(
        "e2b",
        SandboxContainerRef::Container(&reused),
        "1",
        &with_kwargs,
    );
    sandbox.arun_code(&mut internal).await.unwrap();
    let recorded = http.requests();
    assert_eq!(recorded.len(), 4_usize);
    assert_eq!(recorded[1], recorded[3]);
}

#[tokio::test]
async fn internal_kwargs_do_not_reach_the_delete_request() {
    let with_kwargs = internal_kwargs();
    let http = FakeHttp::new(&[(200, ""), (200, "")]);
    let sandbox = build_dispatch(&http, Environment::new());
    let kwargs = empty_kwargs();
    let api_key = secret("e2b-key");
    let mut plain = delete_request("e2b", SandboxContainerRef::Id("sbx_1"), &kwargs);
    plain.call.api_key = Some(&api_key);
    assert!(sandbox.adelete_sandbox(&mut plain).await.unwrap());
    let mut internal = delete_request("e2b", SandboxContainerRef::Id("sbx_1"), &with_kwargs);
    internal.call.api_key = Some(&api_key);
    assert!(sandbox.adelete_sandbox(&mut internal).await.unwrap());
    let recorded = http.requests();
    assert_eq!(recorded.len(), 2_usize);
    assert_eq!(recorded[0], recorded[1]);
}

fn internal_kwargs() -> IndexMap<String, Json> {
    let mut kwargs = IndexMap::new();
    kwargs.insert(
        "litellm_logging_obj".to_owned(),
        Json::Text("obj".to_owned()),
    );
    kwargs.insert("litellm_call_id".to_owned(), Json::Text("call".to_owned()));
    kwargs.insert(
        "litellm_trace_id".to_owned(),
        Json::Text("trace".to_owned()),
    );
    kwargs.insert("litellm_metadata".to_owned(), Json::Object(IndexMap::new()));
    kwargs
}

#[tokio::test]
async fn run_forwards_env_vars_and_uses_the_access_token() {
    let http = FakeHttp::new(&[E2B_CREATED, EXECUTED]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("e2b-key");
    let kwargs = empty_kwargs();
    let mut create_call = create_request("e2b", Some(&api_key), &kwargs);
    let created = sandbox.acreate_sandbox(&mut create_call).await.unwrap();
    let env_vars = BTreeMap::from([("MODE".to_owned(), "test".to_owned())]);
    let mut run = run_request(
        "e2b",
        SandboxContainerRef::Container(&created),
        "print(1)",
        &kwargs,
    );
    run.env_vars = Some(&env_vars);
    let result = sandbox.arun_code(&mut run).await.unwrap();
    assert_eq!(result.stdout, "hello");
    let recorded = http.requests();
    assert_eq!(recorded[1].url, "https://49999-sbx_1.e2b.app/execute");
    assert_eq!(header(&recorded[1], "x-access-token"), "envd-token");
    assert_eq!(
        body(&recorded[1]),
        r#"{"code":"print(1)","context_id":null,"env_vars":{"MODE":"test"}}"#
    );
}

#[tokio::test]
async fn run_from_an_opensandbox_handle_does_not_fetch_the_endpoint_again() {
    let http = FakeHttp::new(&[
        (200, r#"{"id":"sbx_9","status":{"state":"Running"}}"#),
        (200, r#"{"endpoint":"http://127.0.0.1:44772","headers":{}}"#),
        (200, "{\"type\":\"stdout\",\"text\":\"hello\"}"),
    ]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("open-key");
    let kwargs = empty_kwargs();
    let mut create_call = create_request("opensandbox", Some(&api_key), &kwargs);
    create_call.api_base = Some("https://open.test");
    let created = sandbox.acreate_sandbox(&mut create_call).await.unwrap();
    let mut run = run_request(
        "opensandbox",
        SandboxContainerRef::Container(&created),
        "1+1",
        &kwargs,
    );
    run.language = Some("bash");
    sandbox.arun_code(&mut run).await.unwrap();
    let recorded = http.requests();
    assert_eq!(recorded.len(), 3_usize);
    assert_eq!(recorded[2].method, "POST");
    assert_eq!(recorded[2].url, "http://127.0.0.1:44772/code");
    assert_eq!(
        body(&recorded[2]),
        r#"{"code":"1+1","context":{"language":"bash"}}"#
    );
}

#[tokio::test]
async fn run_code_with_a_bare_id_needs_the_e2b_handle() {
    let http = FakeHttp::new(&[]);
    let sandbox = build_dispatch(&http, Environment::new());
    let kwargs = empty_kwargs();
    let mut request = run_request("e2b", SandboxContainerRef::Id("sbx_1"), "1", &kwargs);
    let error = sandbox.arun_code(&mut request).await.unwrap_err();
    assert!(matches!(
        error,
        SandboxLifecycleError::ContainerHandleRequired { provider: "e2b" }
    ));
    assert!(http.requests().is_empty());
}

#[tokio::test]
async fn delete_returns_false_when_e2b_answers_404() {
    let http = FakeHttp::new(&[(404, "")]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("e2b-key");
    let kwargs = empty_kwargs();
    let mut request = delete_request("e2b", SandboxContainerRef::Id("sbx_1"), &kwargs);
    request.call.api_key = Some(&api_key);
    request.api_base = Some("https://e2b.test");
    assert!(!sandbox.adelete_sandbox(&mut request).await.unwrap());
    let sent = http.only_request();
    assert_eq!(sent.method, "DELETE");
    assert_eq!(sent.url, "https://e2b.test/sandboxes/sbx_1");
    assert_eq!(header(&sent, "x-api-key"), "e2b-key");
}

#[tokio::test]
async fn delete_forwards_an_opensandbox_id_and_returns_true() {
    let http = FakeHttp::new(&[(200, "")]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("open-key");
    let kwargs = empty_kwargs();
    let mut request = delete_request("opensandbox", SandboxContainerRef::Id("sbx_9"), &kwargs);
    request.call.api_key = Some(&api_key);
    request.api_base = Some("https://open.test");
    assert!(sandbox.adelete_sandbox(&mut request).await.unwrap());
    let sent = http.only_request();
    assert_eq!(sent.method, "DELETE");
    assert_eq!(sent.url, "https://open.test/sandboxes/sbx_9");
    assert_eq!(header(&sent, "open-sandbox-api-key"), "open-key");
}

#[tokio::test]
async fn delete_returns_false_when_opensandbox_answers_404() {
    let http = FakeHttp::new(&[(404, "")]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("open-key");
    let kwargs = empty_kwargs();
    let mut request = delete_request("opensandbox", SandboxContainerRef::Id("sbx_9"), &kwargs);
    request.call.api_key = Some(&api_key);
    request.api_base = Some("https://open.test");
    assert!(!sandbox.adelete_sandbox(&mut request).await.unwrap());
    let sent = http.only_request();
    assert_eq!(sent.method, "DELETE");
    assert_eq!(sent.url, "https://open.test/sandboxes/sbx_9");
}

#[tokio::test]
async fn the_logging_state_impl_records_the_operation_through_update_from_kwargs() {
    let http = FakeHttp::new(&[E2B_CREATED]);
    let sandbox = build_dispatch(&http, Environment::new());
    let api_key = secret("e2b-key");
    let mut kwargs = empty_kwargs();
    kwargs.insert(
        "litellm_call_id".to_owned(),
        Json::Text("call-3".to_owned()),
    );
    let mut logging = logging_state();
    let mut request = create_request("e2b", Some(&api_key), &kwargs);
    request.call.logging = Some(&mut logging);
    sandbox.acreate_sandbox(&mut request).await.unwrap();
    assert_eq!(logging.model, "e2b/create_sandbox");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("e2b"));
    assert!(logging.optional_params.is_empty());
    assert_eq!(
        logging.litellm_params.get("litellm_call_id"),
        Some(&Json::Text("call-3".to_owned()))
    );
}

fn logging_state() -> LoggingState {
    let start = DateTime::<FixedOffset>::parse_from_rfc3339("2026-01-01T00:00:00+00:00").unwrap();
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "sandbox".to_owned(),
            call_type: "acreate_sandbox".to_owned(),
            start_time: start,
            litellm_call_id: "call-0".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: None,
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
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .unwrap()
}

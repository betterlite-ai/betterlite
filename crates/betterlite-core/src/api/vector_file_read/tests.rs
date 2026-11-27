#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::api::vector_file_read::loads_read_timeout;
use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoreFilesConfigTag;
use betterlite_error::envelope::WireField;
use betterlite_http::proxy::UpstreamResponse;
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
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::{HeaderMap, Method};
use rstest::rstest;
use secrecy::SecretString;

use super::error::{SdkFailure, VectorFileReadError, failure};
use super::service::{
    ReadCall, ReadSession, SdkDeps, VectorStoreFilesConfigs, entry_from_flag, handler_call,
};
use crate::api::vector_file_http_read::config::{OpenAiReadConfig, ReadConfig, ReadParams};
use crate::api::vector_file_http_read::service::{
    ClientSource, LogContext, Operation, ReadRequest, ReadTransport,
};
use crate::api::vector_file_provider::{CredentialLookup, CredentialLookupError, OperationKwargs};
use crate::api::vector_store_http_write::error::ClientError;
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

const RETRIEVE_BODY: &str = r#"{"id":"file_a","object":"vector_store.file","status":"completed"}"#;
const CONTENT_BODY: &str = r#"{"file_id":"file_a","filename":"a","attributes":{"tag":"x"},"content":[{"type":"text","text":"hello"},{"type":"text","text":"world"}]}"#;
const MISSING_BODY: &str = r#"{"error":{"message":"missing","type":"invalid_request_error","param":null,"code":"missing"}}"#;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

fn params(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the params fixture is valid")
}

type Answer = (u16, HeaderMap, Vec<u8>);

#[derive(Default)]
struct Seen {
    requests: Vec<ReadRequest>,
    ssl_verifies: Vec<Option<bool>>,
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Seen>>);

impl Shared {
    fn with<R, F>(&self, read: F) -> R
    where
        F: FnOnce(&mut Seen) -> R,
    {
        read(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

struct Source {
    seen: Shared,
    answer: Answer,
}

impl ClientSource for Source {
    type Client = FakeClient;

    fn client(&self, ssl_verify: Option<bool>) -> Result<FakeClient, ClientError> {
        self.seen.with(|seen| seen.ssl_verifies.push(ssl_verify));
        Ok(FakeClient {
            answer: self.answer.clone(),
            seen: self.seen.clone(),
        })
    }
}

struct FakeClient {
    answer: Answer,
    seen: Shared,
}

impl ReadTransport for FakeClient {
    fn send(
        &self,
        request: ReadRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.seen.with(|seen| seen.requests.push(request));
        let (status, headers, body) = self.answer.clone();
        ready(Ok(UpstreamResponse {
            status,
            headers,
            body,
        }))
    }
}

// the env's OpenAI credentials, exactly as B010's config is wired for the tag
struct Configs {
    credentials: OpenAiCredentialsSettings,
    env_api_key: Option<SecretString>,
}

impl VectorStoreFilesConfigs for Configs {
    fn read_config(&self, tag: VectorStoreFilesConfigTag) -> Option<Box<dyn ReadConfig + '_>> {
        if !matches!(tag, VectorStoreFilesConfigTag::OpenAI) {
            return None;
        }
        Some(Box::new(OpenAiReadConfig::new(
            &self.credentials,
            LiteLlmGlobals::default(),
            self.env_api_key.clone(),
            None,
        )))
    }
}

// the registry read: fixed credential values, or the offline lookup that falls back to the caller's key
struct Credentials {
    values: Result<JsonObject, CredentialLookupError>,
}

impl CredentialLookup for Credentials {
    fn lookup_credentials(
        &self,
        _vector_store_id: &str,
    ) -> Result<OperationKwargs, CredentialLookupError> {
        match &self.values {
            Ok(values) => Ok(OperationKwargs(values.clone())),
            Err(error) => Err(error.clone()),
        }
    }
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

fn env_of<'env>(
    redactor: &'env Redactor,
    sink: &'env Sink,
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
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

struct Logs {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Logs {
    fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"vector-store","call_type":"vector_store_file_retrieve","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
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
            state,
            now,
        }
    }

    fn detail(&self, key: &str) -> Json {
        self.state
            .model_call_details
            .get(key)
            .cloned()
            .unwrap_or(Json::Null)
    }

    fn litellm_params(&self) -> JsonObject {
        match self.detail("litellm_params") {
            Json::Object(fields) => fields,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                panic!("the logged litellm params are an object")
            }
        }
    }
}

struct Fakes {
    configs: Configs,
    registry: Credentials,
    source: Source,
}

impl Fakes {
    fn new(answer: &str) -> Self {
        Self {
            configs: Configs {
                credentials: OpenAiCredentialsSettings::default(),
                env_api_key: None,
            },
            registry: Credentials {
                values: Ok(JsonObject::new()),
            },
            source: Source {
                seen: Shared::default(),
                answer: (200, HeaderMap::new(), answer.as_bytes().to_vec()),
            },
        }
    }

    fn status(&mut self, status: u16, body: &str) {
        self.source.answer = (status, HeaderMap::new(), body.as_bytes().to_vec());
    }

    fn deps(&self) -> SdkDeps<'_, Source> {
        SdkDeps {
            configs: &self.configs,
            registry: Some(&self.registry),
            source: &self.source,
            supplied: None,
            request_timeout_seconds: 6000.0_f64,
        }
    }

    fn requests(&self) -> Vec<ReadRequest> {
        self.source.seen.with(|seen| seen.requests.clone())
    }

    fn ssl_verifies(&self) -> Vec<Option<bool>> {
        self.source.seen.with(|seen| seen.ssl_verifies.clone())
    }
}

#[derive(Clone, Copy)]
enum Which {
    Retrieve,
    ARetrieve,
    Content,
    AContent,
}

async fn run(
    logs: &mut Logs,
    deps: &SdkDeps<'_, Source>,
    call: &ReadCall<'_>,
    which: Which,
) -> Result<Json, VectorFileReadError> {
    let env = env_of(&logs.redactor, &logs.sink, logs.now);
    let mut log = LogContext {
        state: &mut logs.state,
        env: &env,
    };
    let mut session = ReadSession {
        deps,
        call,
        log: &mut log,
    };
    match which {
        Which::Retrieve => session.retrieve().await,
        Which::ARetrieve => session.aretrieve().await,
        Which::Content => session.retrieve_content().await,
        Which::AContent => session.aretrieve_content().await,
    }
}

fn read_call<'call>(
    kwargs: &'call JsonObject,
    headers: Option<&'call BTreeMap<String, String>>,
    timeout: Option<f64>,
    provider: Option<&'call str>,
) -> ReadCall<'call> {
    ReadCall {
        vector_store_id: "vs_a",
        file_id: "file_a",
        extra_headers: headers,
        timeout,
        custom_llm_provider: provider,
        kwargs,
    }
}

fn header(request: &ReadRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .map(|value| value.to_str().expect("the header is text").to_owned())
}

// one captured GET: the method, the path and no query, on the request and on its URL
fn assert_get(requests: &[ReadRequest], path: &str) {
    assert_eq!(requests.len(), 1);
    let request = requests.first().expect("one request was sent");
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url.path(), path);
    assert!(request.query.is_empty());
    assert_eq!(request.url.query(), None);
}

// the wait a read handler runs with, when the call's other arguments stay at the test's defaults
fn handler_timeout(call: &ReadCall<'_>, request_timeout_seconds: f64) -> Option<Duration> {
    handler_call(
        call,
        ReadParams::default(),
        ClientKind::Sync,
        Operation::Retrieve,
        request_timeout_seconds,
    )
    .timeout
}

fn mapped(error: &VectorFileReadError) -> SdkFailure {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    failure(error, &redactor)
}

async fn failed_read(
    fakes: &mut Fakes,
    kwargs: &JsonObject,
    which: Which,
    status: u16,
    body: &str,
) -> SdkFailure {
    fakes.status(status, body);
    let mut logs = Logs::new();
    let call = read_call(kwargs, None, None, None);
    let deps = fakes.deps();
    let error = run(&mut logs, &deps, &call, which)
        .await
        .expect_err("the read fails");
    mapped(&error)
}

fn assert_null_fields(projected: &SdkFailure) {
    assert_eq!(projected.kind, WireField::Null);
    assert_eq!(projected.param, WireField::Null);
    assert_eq!(projected.code, WireField::Null);
}

#[tokio::test]
async fn retrieve_attachment_and_text() {
    let headers = BTreeMap::from([(String::from("X-Trace"), String::from("t"))]);
    let kwargs = params(
        r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller","litellm_call_id":"call-a"}"#,
    );
    let fakes = Fakes::new(RETRIEVE_BODY);
    let mut logs = Logs::new();
    let call = read_call(&kwargs, Some(&headers), None, None);
    let deps = fakes.deps();
    let answer = run(&mut logs, &deps, &call, Which::Retrieve)
        .await
        .expect("the mock answers");
    assert_eq!(answer, json(RETRIEVE_BODY));
    let requests = fakes.requests();
    assert_get(&requests, "/v1/vector_stores/vs_a/files/file_a");
    assert_eq!(
        header(&requests[0], "authorization"),
        Some(String::from("Bearer sk-caller"))
    );
    assert_eq!(header(&requests[0], "x-trace"), Some(String::from("t")));
    assert_eq!(
        header(&requests[0], "content-type"),
        Some(String::from("application/json"))
    );

    let text_fakes = Fakes::new(CONTENT_BODY);
    let mut text_logs = Logs::new();
    let text_call = read_call(&kwargs, Some(&headers), None, None);
    let text_deps = text_fakes.deps();
    let text_answer = run(&mut text_logs, &text_deps, &text_call, Which::AContent)
        .await
        .expect("the mock answers");
    assert_eq!(text_answer, json(CONTENT_BODY));
    let text_requests = text_fakes.requests();
    assert_get(
        &text_requests,
        "/v1/vector_stores/vs_a/files/file_a/content",
    );
}

#[test]
fn async_flag_truthiness() {
    let flag = "aretrieve";
    assert_eq!(
        entry_from_flag(&params(r#"{"aretrieve":true}"#), flag),
        ClientKind::Asynchronous
    );
    assert_eq!(
        entry_from_flag(&params(r#"{"aretrieve":1}"#), flag),
        ClientKind::Sync
    );
    assert_eq!(
        entry_from_flag(&params(r#"{"aretrieve":"true"}"#), flag),
        ClientKind::Sync
    );
    assert_eq!(entry_from_flag(&JsonObject::new(), flag), ClientKind::Sync);
}

#[tokio::test]
async fn content_has_no_pagination() {
    let kwargs = params(
        r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller","litellm_call_id":"call-a","after":"cursor","limit":1,"extra_query":{"trace":"ignored"},"removed":null}"#,
    );
    let fakes = Fakes::new(CONTENT_BODY);
    let mut logs = Logs::new();
    let call = read_call(&kwargs, None, None, None);
    let deps = fakes.deps();
    let answer = run(&mut logs, &deps, &call, Which::Content)
        .await
        .expect("the mock answers");
    let requests = fakes.requests();
    assert_get(&requests, "/v1/vector_stores/vs_a/files/file_a/content");
    assert_eq!(answer, json(CONTENT_BODY));
    assert_eq!(
        logs.detail("optional_params"),
        json(r#"{"vector_store_id":"vs_a","file_id":"file_a"}"#)
    );
    let litellm_params = logs.litellm_params();
    assert_eq!(
        litellm_params.get("after"),
        Some(&Json::Text(String::from("cursor")))
    );
    assert_eq!(
        litellm_params.get("limit"),
        Some(&Json::Number(serde_json::Number::from(1_i32)))
    );
    assert_eq!(
        litellm_params.get("extra_query"),
        Some(&json(r#"{"trace":"ignored"}"#))
    );
    assert_eq!(litellm_params.get("removed"), None);
    assert_eq!(
        litellm_params.get("use_litellm_proxy"),
        Some(&Json::Bool(false))
    );
}

// update_from_kwargs receives the ids first, then GenericLiteLLMParams' dump
#[tokio::test]
async fn litellm_params_start_with_the_ids() {
    let kwargs = params(
        r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller","litellm_call_id":"call-a","after":"cursor"}"#,
    );
    let fakes = Fakes::new(CONTENT_BODY);
    let mut logs = Logs::new();
    let call = read_call(&kwargs, None, None, None);
    let deps = fakes.deps();
    run(&mut logs, &deps, &call, Which::Content)
        .await
        .expect("the mock answers");
    let litellm_params = logs.litellm_params();
    let keys: Vec<&String> = litellm_params.keys().take(2).collect();
    assert_eq!(
        keys,
        vec![
            &String::from("vector_store_id"),
            &String::from("litellm_call_id")
        ]
    );
}

#[tokio::test]
async fn async_calls_keep_the_callers_context() {
    let kwargs =
        params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller","aretrieve":true}"#);
    let fakes = Fakes::new(RETRIEVE_BODY);
    let mut logs = Logs::new();
    let call = read_call(&kwargs, None, None, None);
    let deps = fakes.deps();
    let answer = run(&mut logs, &deps, &call, Which::ARetrieve)
        .await
        .expect("the mock answers");
    assert_eq!(answer, json(RETRIEVE_BODY));
    assert_eq!(logs.state.litellm_trace_id, "trace-1");
    assert_eq!(
        logs.detail("model"),
        Json::Text(String::from("vector-store"))
    );
    assert_eq!(
        logs.detail("custom_llm_provider"),
        Json::Text(String::from("openai"))
    );
    // the popped flag never reaches the logged params, and an absent call id is logged as null
    let litellm_params = logs.litellm_params();
    assert_eq!(litellm_params.get("litellm_call_id"), Some(&Json::Null));
    assert_eq!(litellm_params.get("aretrieve"), None);
}

// the client a read takes: one supplied in its own kind, the pool for every other call
#[rstest]
#[case::retrieve_sync(Which::Retrieve, None, ClientKind::Sync, true)]
#[case::retrieve_sync_drops_async(Which::Retrieve, None, ClientKind::Asynchronous, false)]
#[case::retrieve_flag(Which::Retrieve, Some("aretrieve"), ClientKind::Asynchronous, true)]
#[case::retrieve_flag_drops_sync(Which::Retrieve, Some("aretrieve"), ClientKind::Sync, false)]
#[case::content_sync(Which::Content, None, ClientKind::Sync, true)]
#[case::content_sync_drops_async(Which::Content, None, ClientKind::Asynchronous, false)]
#[case::content_flag(
    Which::Content,
    Some("aretrieve_content"),
    ClientKind::Asynchronous,
    true
)]
#[case::content_flag_drops_sync(Which::Content, Some("aretrieve_content"), ClientKind::Sync, false)]
#[case::aretrieve(Which::ARetrieve, None, ClientKind::Asynchronous, true)]
#[case::aretrieve_drops_sync(Which::ARetrieve, None, ClientKind::Sync, false)]
#[case::acontent(Which::AContent, None, ClientKind::Asynchronous, true)]
#[case::acontent_drops_sync(Which::AContent, None, ClientKind::Sync, false)]
#[tokio::test]
async fn reads_reuse_only_a_client_of_their_kind(
    #[case] which: Which,
    #[case] flag: Option<&'static str>,
    #[case] supplied_kind: ClientKind,
    #[case] reused: bool,
) {
    let mut kwargs = params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller"}"#);
    if let Some(name) = flag {
        kwargs.insert(String::from(name), Json::Bool(true));
    }
    let fakes = Fakes::new(RETRIEVE_BODY);
    let supplied = FakeClient {
        answer: (200, HeaderMap::new(), CONTENT_BODY.as_bytes().to_vec()),
        seen: Shared::default(),
    };
    let deps = SdkDeps {
        configs: &fakes.configs,
        registry: Some(&fakes.registry),
        source: &fakes.source,
        supplied: Some(SuppliedClient {
            kind: supplied_kind,
            transport: &supplied,
        }),
        request_timeout_seconds: 6000.0_f64,
    };
    let mut logs = Logs::new();
    let call = read_call(&kwargs, None, None, None);
    let answer = run(&mut logs, &deps, &call, which)
        .await
        .expect("the mock answers");
    let supplied_requests = supplied.seen.with(|seen| seen.requests.len());
    let expected = if reused {
        json(CONTENT_BODY)
    } else {
        json(RETRIEVE_BODY)
    };
    assert_eq!(answer, expected);
    assert_eq!(fakes.requests().len(), usize::from(!reused));
    assert_eq!(supplied_requests, usize::from(reused));
}

// the pool client follows ssl_verify, and a registry value overwrites the caller's
#[rstest]
#[case::retrieve_default(Which::Retrieve, None, None, None)]
#[case::retrieve_verify(Which::Retrieve, Some(true), None, Some(true))]
#[case::retrieve_no_verify(Which::Retrieve, Some(false), None, Some(false))]
#[case::content_verify(Which::Content, Some(true), None, Some(true))]
#[case::content_no_verify(Which::Content, Some(false), None, Some(false))]
#[case::registry_wins(Which::Retrieve, Some(false), Some(true), Some(true))]
#[tokio::test]
async fn a_pool_client_follows_ssl_verify(
    #[case] which: Which,
    #[case] ssl_verify: Option<bool>,
    #[case] registry_value: Option<bool>,
    #[case] expected: Option<bool>,
) {
    let mut kwargs = params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller"}"#);
    if let Some(value) = ssl_verify {
        kwargs.insert(String::from("ssl_verify"), Json::Bool(value));
    }
    let mut fakes = Fakes::new(RETRIEVE_BODY);
    if let Some(value) = registry_value {
        fakes.registry = Credentials {
            values: Ok(JsonObject::from([(
                String::from("ssl_verify"),
                Json::Bool(value),
            )])),
        };
    }
    let mut logs = Logs::new();
    let call = read_call(&kwargs, None, None, None);
    let deps = fakes.deps();
    run(&mut logs, &deps, &call, which)
        .await
        .expect("the mock answers");
    assert_eq!(fakes.ssl_verifies(), vec![expected]);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn read_registry_and_provider(#[case] provider: Option<&'static str>) {
    let kwargs = params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller"}"#);
    let mut fakes = Fakes::new(RETRIEVE_BODY);
    fakes.registry = Credentials {
        values: Ok(JsonObject::from([(
            String::from("api_key"),
            Json::Text(String::from("sk-registry")),
        )])),
    };
    let mut logs = Logs::new();
    let call = read_call(&kwargs, None, None, provider);
    let deps = fakes.deps();
    run(&mut logs, &deps, &call, Which::Retrieve)
        .await
        .expect("the mock answers");
    assert_eq!(
        fakes.requests()[0].url.path(),
        "/v1/vector_stores/vs_a/files/file_a"
    );
    assert_eq!(
        header(&fakes.requests()[0], "authorization"),
        Some(String::from("Bearer sk-registry"))
    );
    assert_eq!(
        logs.detail("custom_llm_provider"),
        Json::Text(String::from("openai"))
    );
}

#[tokio::test]
async fn registry_provider_field_cannot_redirect_the_read() {
    let kwargs = params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller"}"#);
    let mut fakes = Fakes::new(RETRIEVE_BODY);
    fakes.registry = Credentials {
        values: Ok(JsonObject::from([
            (
                String::from("api_key"),
                Json::Text(String::from("sk-registry")),
            ),
            (
                String::from("custom_llm_provider"),
                Json::Text(String::from("azure")),
            ),
        ])),
    };
    let mut logs = Logs::new();
    let call = read_call(&kwargs, None, None, None);
    let deps = fakes.deps();
    run(&mut logs, &deps, &call, Which::Retrieve)
        .await
        .expect("the mock answers");
    assert_eq!(
        header(&fakes.requests()[0], "authorization"),
        Some(String::from("Bearer sk-registry"))
    );
    assert_eq!(
        logs.detail("custom_llm_provider"),
        Json::Text(String::from("openai"))
    );
    // the overlay still lands in the logged params, where LiteLLM records the kwargs the caller sent
    assert_eq!(
        logs.litellm_params().get("custom_llm_provider"),
        Some(&Json::Text(String::from("azure")))
    );
}

#[tokio::test]
async fn registry_lookup_failure_falls_back_to_the_caller_key() {
    let kwargs = params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller"}"#);
    let mut fakes = Fakes::new(RETRIEVE_BODY);
    fakes.registry = Credentials {
        values: Err(CredentialLookupError::Unavailable {
            reason: String::from("offline"),
        }),
    };
    let mut logs = Logs::new();
    let call = read_call(&kwargs, None, None, None);
    let deps = fakes.deps();
    run(&mut logs, &deps, &call, Which::Retrieve)
        .await
        .expect("the mock answers");
    assert_eq!(
        header(&fakes.requests()[0], "authorization"),
        Some(String::from("Bearer sk-caller"))
    );
}

// the exception an unsupported provider raises: its message and the APIConnectionError shape
fn assert_unsupported_failure(error: &VectorFileReadError, message: &str) {
    assert_eq!(error.to_string(), message);
    let projected = mapped(error);
    assert_eq!(projected.class, "APIConnectionError");
    assert_eq!(projected.status, 500);
    assert_null_fields(&projected);
}

#[tokio::test]
async fn unsupported_providers_fail_before_http() {
    let kwargs = params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller"}"#);
    let fakes = Fakes::new(RETRIEVE_BODY);
    let mut logs = Logs::new();
    let azure_call = read_call(&kwargs, None, None, Some("azure"));
    let deps = fakes.deps();
    let azure_error = run(&mut logs, &deps, &azure_call, Which::Retrieve)
        .await
        .expect_err("azure serves no file read");
    assert_unsupported_failure(
        &azure_error,
        "Vector store file retrieve is not supported for azure",
    );
    let content_call = read_call(&kwargs, None, None, Some("azure"));
    let content_error = run(&mut logs, &deps, &content_call, Which::Content)
        .await
        .expect_err("azure serves no file content");
    assert_unsupported_failure(
        &content_error,
        "Vector store file content retrieve is not supported for azure",
    );
    let bogus_call = read_call(&kwargs, None, None, Some("bogus"));
    let bogus_error = run(&mut logs, &deps, &bogus_call, Which::Retrieve)
        .await
        .expect_err("bogus is no provider");
    assert_unsupported_failure(&bogus_error, "'bogus' is not a valid LlmProviders");
    assert!(fakes.requests().is_empty());
}

#[tokio::test]
async fn retrieve_missing_file() {
    let kwargs =
        params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller","max_retries":0}"#);
    let mut fakes = Fakes::new(RETRIEVE_BODY);
    let retrieve_failure =
        failed_read(&mut fakes, &kwargs, Which::Retrieve, 404, MISSING_BODY).await;
    assert_eq!(retrieve_failure.class, "NotFoundError");
    assert_eq!(retrieve_failure.status, 404);
    assert_null_fields(&retrieve_failure);
    assert_eq!(fakes.requests().len(), 1);
    let mut content_src = Fakes::new(CONTENT_BODY);
    let content_failure =
        failed_read(&mut content_src, &kwargs, Which::Content, 404, MISSING_BODY).await;
    assert_eq!(content_failure.class, "NotFoundError");
    assert_eq!(content_failure.status, 404);
    assert_null_fields(&content_failure);
    assert_eq!(content_src.requests().len(), 1);
}

#[tokio::test]
async fn an_empty_missing_body_maps_by_status() {
    let kwargs =
        params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller","max_retries":0}"#);
    let mut fakes = Fakes::new(RETRIEVE_BODY);
    let missing_failure = failed_read(&mut fakes, &kwargs, Which::Retrieve, 404, "").await;
    assert_eq!(missing_failure.class, "NotFoundError");
    assert_eq!(missing_failure.status, 404);
    assert_null_fields(&missing_failure);
}

#[test]
fn read_timeout_argument() {
    let kwargs = JsonObject::new();
    let mut call = read_call(&kwargs, None, None, None);
    assert_eq!(
        handler_timeout(&call, 6000.0_f64),
        Some(Duration::from_mins(100))
    );
    call.timeout = Some(0.0_f64);
    assert_eq!(
        handler_timeout(&call, 6000.0_f64),
        Some(Duration::from_mins(100))
    );
    call.timeout = Some(7.0_f64);
    assert_eq!(
        handler_timeout(&call, 6000.0_f64),
        Some(Duration::from_secs(7))
    );

    let environment = BTreeMap::from([(String::from("REQUEST_TIMEOUT"), String::from("12"))]);
    let configured = loads_read_timeout(&environment).expect("the environment value loads");
    call.timeout = None;
    assert_eq!(
        handler_timeout(&call, configured),
        Some(Duration::from_secs(12))
    );
    call.timeout = Some(0.0_f64);
    assert_eq!(
        handler_timeout(&call, configured),
        Some(Duration::from_secs(12))
    );
    call.timeout = Some(7.0_f64);
    assert_eq!(
        handler_timeout(&call, configured),
        Some(Duration::from_secs(7))
    );
}

// a negative wait is no wait at all, and the read still runs: LiteLLM forwards it and the GET never uses it
#[tokio::test]
async fn a_negative_timeout_still_reads() {
    let kwargs = params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller"}"#);
    let fakes = Fakes::new(RETRIEVE_BODY);
    let mut logs = Logs::new();
    let mut call = read_call(&kwargs, None, Some(-1.0_f64), None);
    assert_eq!(handler_timeout(&call, 6000.0_f64), None);
    let deps = fakes.deps();
    let answer = run(&mut logs, &deps, &call, Which::Retrieve)
        .await
        .expect("the mock answers");
    assert_eq!(answer, json(RETRIEVE_BODY));
    assert_eq!(fakes.requests().len(), 1);
    let environment = BTreeMap::from([(String::from("REQUEST_TIMEOUT"), String::from("-1"))]);
    let negative_seconds = loads_read_timeout(&environment).expect("a negative value loads");
    call.timeout = None;
    assert_eq!(handler_timeout(&call, negative_seconds), None);
}

#[rstest]
#[case::base(r#"{"api_base":5}"#)]
#[case::key(r#"{"api_base":"http://mock.invalid/v1","api_key":5}"#)]
#[case::ssl(r#"{"api_base":"http://mock.invalid/v1","ssl_verify":"yes"}"#)]
#[tokio::test]
async fn wrong_field_types_name_the_field(#[case] text: &'static str) {
    let fakes = Fakes::new(RETRIEVE_BODY);
    let deps = fakes.deps();
    let kwargs = params(text);
    let mut logs = Logs::new();
    let call = read_call(&kwargs, None, None, None);
    let error = run(&mut logs, &deps, &call, Which::Retrieve)
        .await
        .expect_err("the field type is refused");
    assert!(error.to_string().contains("field must be"));
    let projected_failure = mapped(&error);
    assert_eq!(projected_failure.class, "APIConnectionError");
    assert_eq!(projected_failure.status, 500);
    assert_null_fields(&projected_failure);
    assert!(fakes.requests().is_empty());
}

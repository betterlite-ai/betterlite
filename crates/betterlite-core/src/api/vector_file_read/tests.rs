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
use secrecy::SecretString;

use super::error::{SdkFailure, VectorFileReadError, failure};
use super::service::{
    ReadCall, ReadEntry, SdkDeps, VectorStoreFilesConfigs, entry_from_flag, handler_call,
};
use super::{aretrieve, aretrieve_content, retrieve, retrieve_content};
use crate::api::vector_file_http_read::config::{OpenAiReadConfig, ReadConfig, ReadParams};
use crate::api::vector_file_http_read::service::{
    ClientSource, Operation, ReadRequest, ReadTransport,
};
use crate::api::vector_file_provider::{CredentialLookup, CredentialLookupError, OperationKwargs};
use crate::api::vector_store_http_write::error::ClientError;
use crate::api::vector_store_http_write::service::{ClientKind, LogContext};

const RETRIEVE_BODY: &str = r#"{"id":"file_a","object":"vector_store.file","status":"completed"}"#;
const CONTENT_BODY: &str = r#"{"file_id":"file_a","filename":"a","attributes":{"tag":"x"},"content":[{"type":"text","text":"hello"},{"type":"text","text":"world"}]}"#;
const MISSING_BODY: &str = r#"{"error":{"message":"missing","type":"invalid_request_error","param":null,"code":"missing"}}"#;
const BASE: &str = "http://mock.invalid/v1";

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
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Seen>>);

impl Shared {
    fn with<R>(&self, read: impl FnOnce(&mut Seen) -> R) -> R {
        read(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

struct Source {
    seen: Shared,
    answer: Answer,
}

impl ClientSource for Source {
    type Client = FakeClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<FakeClient, ClientError> {
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
            Duration::default(),
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
            registry: Some(&self.registry as &dyn CredentialLookup),
            source: &self.source,
            supplied: None,
            request_timeout_seconds: 6000.0_f64,
        }
    }

    fn requests(&self) -> Vec<ReadRequest> {
        self.source.seen.with(|seen| seen.requests.clone())
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
    match which {
        Which::Retrieve => retrieve(deps, call, &mut log).await,
        Which::ARetrieve => aretrieve(deps, call, &mut log).await,
        Which::Content => retrieve_content(deps, call, &mut log).await,
        Which::AContent => aretrieve_content(deps, call, &mut log).await,
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

fn assert_null_fields(mapped: &SdkFailure) {
    assert_eq!(mapped.kind, WireField::Null);
    assert_eq!(mapped.param, WireField::Null);
    assert_eq!(mapped.code, WireField::Null);
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
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(
        requests[0].url.path(),
        "/v1/vector_stores/vs_a/files/file_a"
    );
    assert!(requests[0].query.is_empty());
    assert_eq!(requests[0].url.query(), None);
    assert_eq!(
        header(&requests[0], "authorization"),
        Some(String::from("Bearer sk-caller"))
    );
    assert_eq!(header(&requests[0], "x-trace"), Some(String::from("t")));
    assert_eq!(
        header(&requests[0], "content-type"),
        Some(String::from("application/json"))
    );

    let fakes = Fakes::new(CONTENT_BODY);
    let mut logs = Logs::new();
    let call = read_call(&kwargs, Some(&headers), None, None);
    let deps = fakes.deps();
    let answer = run(&mut logs, &deps, &call, Which::AContent)
        .await
        .expect("the mock answers");
    assert_eq!(answer, json(CONTENT_BODY));
    let requests = fakes.requests();
    assert_eq!(
        requests[0].url.path(),
        "/v1/vector_stores/vs_a/files/file_a/content"
    );
    assert!(requests[0].query.is_empty());
    assert_eq!(header(&requests[0], "x-trace"), Some(String::from("t")));
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
    assert_eq!(
        requests[0].url.path(),
        "/v1/vector_stores/vs_a/files/file_a/content"
    );
    assert_eq!(requests[0].url.query(), None);
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
        Some(&Json::Number(serde_json::Number::from(1)))
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
    run(&mut logs, &deps, &call, Which::ARetrieve)
        .await
        .expect("the mock answers");
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

    let fakes = Fakes::new(RETRIEVE_BODY);
    let mut logs = Logs::new();
    let call = read_call(&kwargs, None, None, None);
    let deps = fakes.deps();
    run(&mut logs, &deps, &call, Which::Retrieve)
        .await
        .expect("a non-boolean flag keeps the sync handler");
    assert!(fakes.requests().len() == 1);
}

#[tokio::test]
async fn read_registry_and_provider() {
    for provider in [None, Some("")] {
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

#[tokio::test]
async fn unsupported_providers_fail_before_http() {
    let kwargs = params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller"}"#);
    let fakes = Fakes::new(RETRIEVE_BODY);
    let mut logs = Logs::new();
    let call = read_call(&kwargs, None, None, Some("azure"));
    let deps = fakes.deps();
    let error = run(&mut logs, &deps, &call, Which::Retrieve)
        .await
        .expect_err("azure serves no file read");
    assert_eq!(
        error.to_string(),
        "Vector store file retrieve is not supported for azure"
    );
    let mapped = mapped(&error);
    assert_eq!(mapped.class, "APIConnectionError");
    assert_eq!(mapped.status, 500);
    assert_null_fields(&mapped);
    let call = read_call(&kwargs, None, None, Some("azure"));
    let error = run(&mut logs, &deps, &call, Which::Content)
        .await
        .expect_err("azure serves no file content");
    assert_eq!(
        error.to_string(),
        "Vector store file content retrieve is not supported for azure"
    );
    let call = read_call(&kwargs, None, None, Some("bogus"));
    let error = run(&mut logs, &deps, &call, Which::Retrieve)
        .await
        .expect_err("bogus is no provider");
    assert_eq!(error.to_string(), "'bogus' is not a valid LlmProviders");
    let mapped = mapped(&error);
    assert_eq!(mapped.class, "APIConnectionError");
    assert_eq!(mapped.status, 500);
    assert_null_fields(&mapped);
    assert!(fakes.requests().is_empty());
}

#[tokio::test]
async fn retrieve_missing_file() {
    let kwargs =
        params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller","max_retries":0}"#);
    let mut fakes = Fakes::new(RETRIEVE_BODY);
    let mapped = failed_read(&mut fakes, &kwargs, Which::Retrieve, 404, MISSING_BODY).await;
    assert_eq!(mapped.class, "NotFoundError");
    assert_eq!(mapped.status, 404);
    assert_null_fields(&mapped);
    assert_eq!(fakes.requests().len(), 1);
    let mut fakes = Fakes::new(CONTENT_BODY);
    let mapped = failed_read(&mut fakes, &kwargs, Which::Content, 404, MISSING_BODY).await;
    assert_eq!(mapped.class, "NotFoundError");
    assert_eq!(mapped.status, 404);
    assert_null_fields(&mapped);
    assert_eq!(fakes.requests().len(), 1);
}

#[tokio::test]
async fn an_empty_missing_body_maps_by_status() {
    let kwargs =
        params(r#"{"api_base":"http://mock.invalid/v1","api_key":"sk-caller","max_retries":0}"#);
    let mut fakes = Fakes::new(RETRIEVE_BODY);
    let mapped = failed_read(&mut fakes, &kwargs, Which::Retrieve, 404, "").await;
    assert_eq!(mapped.class, "NotFoundError");
    assert_eq!(mapped.status, 404);
    assert_null_fields(&mapped);
}

#[test]
fn read_timeout_argument() {
    let kwargs = JsonObject::new();
    let entry = ReadEntry {
        operation: Operation::Retrieve,
        mode: ClientKind::Sync,
        kwargs: &kwargs,
    };
    let mut call = read_call(&kwargs, None, None, None);
    let default =
        handler_call(&call, ReadParams::default(), &entry, 6000.0_f64).expect("the default");
    assert_eq!(default.timeout, Duration::from_mins(100));
    call.timeout = Some(0.0_f64);
    let zero = handler_call(&call, ReadParams::default(), &entry, 6000.0_f64)
        .expect("zero takes the default");
    assert_eq!(zero.timeout, Duration::from_mins(100));
    call.timeout = Some(7.0_f64);
    let given =
        handler_call(&call, ReadParams::default(), &entry, 6000.0_f64).expect("the given wait");
    assert_eq!(given.timeout, Duration::from_secs(7));

    let environment = BTreeMap::from([(String::from("REQUEST_TIMEOUT"), String::from("12"))]);
    let configured = loads_read_timeout(&environment).expect("the environment value loads");
    call.timeout = None;
    let from_environment = handler_call(&call, ReadParams::default(), &entry, configured)
        .expect("the configured default");
    assert_eq!(from_environment.timeout, Duration::from_secs(12));
    call.timeout = Some(0.0_f64);
    let zero = handler_call(&call, ReadParams::default(), &entry, configured)
        .expect("zero takes the configured default");
    assert_eq!(zero.timeout, Duration::from_secs(12));
    call.timeout = Some(7.0_f64);
    let given =
        handler_call(&call, ReadParams::default(), &entry, configured).expect("the given wait");
    assert_eq!(given.timeout, Duration::from_secs(7));
}

#[tokio::test]
async fn wrong_field_types_name_the_field() {
    let fakes = Fakes::new(RETRIEVE_BODY);
    let deps = fakes.deps();
    for text in [
        r#"{"api_base":5}"#,
        r#"{"api_base":"http://mock.invalid/v1","api_key":5}"#,
        r#"{"api_base":"http://mock.invalid/v1","ssl_verify":"yes"}"#,
    ] {
        let kwargs = params(text);
        let mut logs = Logs::new();
        let call = read_call(&kwargs, None, None, None);
        let error = run(&mut logs, &deps, &call, Which::Retrieve)
            .await
            .expect_err("the field type is refused");
        assert!(error.to_string().contains("field must be"));
        let mapped = mapped(&error);
        assert_eq!(mapped.class, "APIConnectionError");
        assert_eq!(mapped.status, 500);
        assert_null_fields(&mapped);
    }
    assert!(fakes.requests().is_empty());
}

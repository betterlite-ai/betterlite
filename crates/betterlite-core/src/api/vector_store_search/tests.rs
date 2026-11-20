#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::api::vector_store_search::Settings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_error::envelope::WireField;
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
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::{
    LiteLlmManagedVectorStore, VectorStoreSearchResponse,
};
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset, Utc};
use http::HeaderMap;
use secrecy::ExposeSecret as _;

use super::error::{SdkFailure, VectorStoreSearchError, failure};
use super::executor::{DirectExecutor, EmbeddingExecutorSource, SelectedExecutor};
use super::provider::resolve_provider;
use super::service::{
    CreateCall, Entry, OptionalParamFilters, SdkDeps, SearchCall, VectorStoreConfigs, create,
    entry_from_flag, search,
};
use crate::api::vector_search_execution::config::{
    EmbeddingEntry, Params, QueryEmbeddingExecutor, SearchConfig, SearchQuery, SearchRequest,
    StoreFailure, VectorStoreLogging, VectorStoreResponse,
};
use crate::api::vector_search_execution::error::SearchError;
use crate::api::vector_search_execution::service::signing::{
    SignFuture, SignInput, SignedSearch, SigningSeam,
};
use crate::api::vector_store_http_create_read::config::{CapabilityError, CreateReadConfig};
use crate::api::vector_store_http_write::config::{WriteConfig, WriteParams};
use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, HeaderMap as ErrorHeaderMap, UrlFailure,
};
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, LogContext, SuppliedClient, WriteRequest, WriteTransport,
};
use crate::api::vector_store_registry::store::{VectorStoreCredentials, VectorStoreRegistry};

const CREATE_RESULT: &str = r#"{"id":"vs_a","object":"vector_store"}"#;
const SEARCH_RESULT: &str =
    r#"{"object":"vector_store.search_results.page","search_query":"sample query","data":[]}"#;
const SAMPLE_SEARCH: &str = r#"{"object":"vector_store.search_results.page","search_query":"sample query","data":[{"score":0.95,"content":[{"text":"This is a sample search result from the vector store.","type":"text"}]}]}"#;
const CREATE_FIELDS: [&str; 5] = [
    "name",
    "file_ids",
    "expires_after",
    "chunking_strategy",
    "metadata",
];
const SEARCH_FIELDS: [&str; 4] = [
    "filters",
    "max_num_results",
    "ranking_options",
    "rewrite_query",
];
const CREATE_OPTIONALS: &str = r#"{"name":"notes","file_ids":["file_a"],"expires_after":{"anchor":"last_active_at","days":7},"chunking_strategy":{"type":"auto"},"metadata":{"tag":"x"}}"#;
const SEARCH_OPTIONALS: &str = r#"{"filters":{"type":"eq","key":"tag","value":"x"},"max_num_results":0,"ranking_options":{"ranker":"auto"},"rewrite_query":false}"#;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

fn params(text: &str) -> Params {
    serde_json::from_str(text).expect("the params fixture is valid")
}

#[derive(Default)]
struct Seen {
    requests: Vec<WriteRequest>,
    bodies: Vec<Json>,
    create_api_keys: Vec<Option<String>>,
    search_timeouts: Vec<Option<Duration>>,
    search_litellm_params: Vec<Params>,
    search_optionals: Vec<Params>,
    clients: Vec<String>,
    executor_supplied: Vec<bool>,
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Seen>>);

impl Shared {
    fn with<R>(&self, read: impl FnOnce(&mut Seen) -> R) -> R {
        read(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

type Answer = (u16, HeaderMap, Vec<u8>);

struct Source {
    seen: Shared,
    answer: Answer,
}

impl ClientSource for Source {
    type Client = FakeClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<FakeClient, ClientError> {
        Ok(FakeClient {
            name: String::from("created"),
            answer: self.answer.clone(),
            seen: self.seen.clone(),
        })
    }
}

struct FakeClient {
    name: String,
    answer: Answer,
    seen: Shared,
}

impl WriteTransport for FakeClient {
    fn default_timeout(&self) -> Duration {
        Duration::from_secs(9)
    }

    fn send(
        &self,
        request: WriteRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let posted = request.method == http::Method::POST;
        self.seen.with(|seen| {
            seen.clients.push(self.name.clone());
            if posted {
                seen.search_timeouts.push(request.timeout);
            }
            seen.bodies.push(match &request.body {
                RequestBody::Json(bytes) => {
                    serde_json::from_slice(bytes).expect("the request body is JSON")
                }
                RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                    Json::Null
                }
            });
            seen.requests.push(request);
        });
        let (status, headers, body) = self.answer.clone();
        ready(Ok(UpstreamResponse {
            status,
            headers,
            body,
        }))
    }
}

struct Configs {
    seen: Shared,
}

impl VectorStoreConfigs for Configs {
    fn create_config(&self, tag: VectorStoresConfigTag) -> Option<Box<dyn CreateReadConfig + '_>> {
        match tag {
            VectorStoresConfigTag::OpenAI => Some(Box::new(CreateConfig {
                seen: self.seen.clone(),
            })),
            VectorStoresConfigTag::AzureOpenAI
            | VectorStoresConfigTag::Vertex
            | VectorStoresConfigTag::VertexSearchAPI
            | VectorStoresConfigTag::Bedrock
            | VectorStoresConfigTag::PGVector
            | VectorStoresConfigTag::AzureAI
            | VectorStoresConfigTag::Milvus
            | VectorStoresConfigTag::Gemini
            | VectorStoresConfigTag::RAGFlow
            | VectorStoresConfigTag::S3Vectors
            | VectorStoresConfigTag::Valkey
            | VectorStoresConfigTag::MongoDB => None,
        }
    }

    fn search_config(&self, tag: VectorStoresConfigTag) -> Option<Arc<dyn SearchConfig>> {
        match tag {
            VectorStoresConfigTag::OpenAI => Some(Arc::new(SearchConfigFake {
                seen: self.seen.clone(),
            })),
            VectorStoresConfigTag::AzureOpenAI
            | VectorStoresConfigTag::Vertex
            | VectorStoresConfigTag::VertexSearchAPI
            | VectorStoresConfigTag::Bedrock
            | VectorStoresConfigTag::PGVector
            | VectorStoresConfigTag::AzureAI
            | VectorStoresConfigTag::Milvus
            | VectorStoresConfigTag::Gemini
            | VectorStoresConfigTag::RAGFlow
            | VectorStoresConfigTag::S3Vectors
            | VectorStoresConfigTag::Valkey
            | VectorStoresConfigTag::MongoDB => None,
        }
    }
}

struct CreateConfig {
    seen: Shared,
}

impl CreateReadConfig for CreateConfig {
    fn validate_create(&self) -> Result<(), CapabilityError> {
        Ok(())
    }

    fn provider(&self) -> &dyn WriteConfig {
        self
    }
}

impl WriteConfig for CreateConfig {
    fn validate_environment(&self, headers: OpenAiHeaders, params: &WriteParams) -> OpenAiHeaders {
        self.seen.with(|seen| {
            seen.create_api_keys.push(
                params
                    .api_key
                    .as_ref()
                    .map(|key| key.expose_secret().to_owned()),
            );
        });
        headers
    }

    fn complete_url(&self, params: &WriteParams) -> Result<String, UrlFailure> {
        Ok(params
            .api_base
            .clone()
            .unwrap_or_else(|| String::from("https://mock.invalid/v1")))
    }

    fn transform_create_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException> {
        serde_json::from_slice(response.body()).map_err(|source| {
            self.error_class(
                &source.to_string(),
                response.status().as_u16(),
                ErrorHeaderMap::new(),
            )
        })
    }

    fn error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: ErrorHeaderMap,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

struct SearchConfigFake {
    seen: Shared,
}

#[async_trait]
impl SearchConfig for SearchConfigFake {
    fn validate_environment(
        &self,
        headers: OpenAiHeaders,
        _litellm_params: &Params,
    ) -> Result<OpenAiHeaders, SearchError> {
        Ok(headers)
    }

    fn complete_url(&self, litellm_params: &Params) -> Result<String, SearchError> {
        match litellm_params.get("api_base") {
            Some(Json::Text(base)) => Ok(base.clone()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => Ok(String::from("https://mock.invalid/v1")),
        }
    }

    fn transform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<(String, Params), SearchError> {
        self.seen.with(|seen| {
            seen.search_litellm_params
                .push(request.litellm_params.clone());
            seen.search_optionals.push(request.optional_params.clone());
        });
        Ok((
            String::from("https://mock.invalid/v1/vector_stores/vs_a/search"),
            request.litellm_params.clone(),
        ))
    }

    fn transform_search_response(
        &self,
        response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, SearchError> {
        serde_json::from_slice(&response.body)
            .map_err(|source| {
                self.error_class(
                    &source.to_string(),
                    response.status_code,
                    ErrorHeaderMap::new(),
                )
            })
            .map_err(SearchError::Provider)
    }

    fn error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: ErrorHeaderMap,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

struct Filters;

impl OptionalParamFilters for Filters {
    fn create_params(&self, params: &Params) -> Params {
        declared(params, &CREATE_FIELDS)
    }

    fn search_params(&self, params: &Params, _tag: VectorStoresConfigTag) -> Params {
        declared(params, &SEARCH_FIELDS)
    }
}

// B1058 over an identity map_params: the declared keys whose values are not null
fn declared(params: &Params, names: &[&str]) -> Params {
    params
        .iter()
        .filter(|(name, value)| names.contains(&name.as_str()) && !matches!(value, Json::Null))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

struct Executors {
    seen: Shared,
}

impl EmbeddingExecutorSource for Executors {
    fn select<'selection>(
        &'selection self,
        supplied: Option<&'selection dyn QueryEmbeddingExecutor>,
        request_kwargs: &Params,
    ) -> SelectedExecutor<'selection> {
        self.seen
            .with(|seen| seen.executor_supplied.push(supplied.is_some()));
        match supplied {
            Some(executor) => SelectedExecutor::Supplied(executor),
            None => SelectedExecutor::Built(Box::new(RecordingExecutor {
                _request_kwargs: request_kwargs.clone(),
            })),
        }
    }
}

struct RecordingExecutor {
    _request_kwargs: Params,
}

#[async_trait]
impl QueryEmbeddingExecutor for RecordingExecutor {
    async fn embed(
        &self,
        _entry: EmbeddingEntry,
        _model: &str,
        _query: &str,
        _configuration: &Params,
    ) -> Result<EmbeddingResponse, StoreFailure> {
        Err(StoreFailure::from(IoError::other(
            "the recording executor embeds nothing",
        )))
    }
}

struct Inline;

impl SigningSeam for Inline {
    fn sign(&self, config: Arc<dyn SearchConfig>, request: SignInput) -> SignFuture<'_> {
        let signed: SignedSearch = config.sign_request(request.request());
        Box::pin(ready(Ok(signed)))
    }
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

fn pre_post_env<'env>(
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
    fn new(call_type: &str) -> Self {
        let init: LoggingInit = serde_json::from_str(&format!(
            r#"{{"model":"vector-store","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
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

    async fn run_create<S>(
        &mut self,
        deps: &SdkDeps<'_, S>,
        call: &CreateCall<'_>,
    ) -> Result<Json, VectorStoreSearchError>
    where
        S: ClientSource,
    {
        let env = pre_post_env(&self.redactor, &self.sink, self.now);
        create(
            deps,
            call,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
        .await
    }

    async fn run_search<S>(
        &mut self,
        deps: &SdkDeps<'_, S>,
        call: &SearchCall<'_>,
    ) -> Result<Json, VectorStoreSearchError>
    where
        S: ClientSource,
    {
        let env = pre_post_env(&self.redactor, &self.sink, self.now);
        search(
            deps,
            call,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
        .await
    }

    fn detail(&self, key: &str) -> Json {
        self.state
            .model_call_details
            .get(key)
            .cloned()
            .unwrap_or(Json::Null)
    }
}

fn settings() -> Settings {
    Settings {
        request_timeout_seconds: 6000.0,
    }
}

struct FixedCredentials(JsonObject);

impl VectorStoreCredentials for FixedCredentials {
    fn credential_values(&self, _credential_name: &str) -> JsonObject {
        self.0.clone()
    }
}

fn managed_store(credential_name: Option<&str>) -> LiteLlmManagedVectorStore {
    LiteLlmManagedVectorStore {
        vector_store_id: Some(String::from("vs_a")),
        litellm_credential_name: credential_name.map(str::to_owned),
        ..LiteLlmManagedVectorStore::default()
    }
}

struct Fakes {
    seen: Shared,
    configs: Configs,
    filters: Filters,
    executors: Executors,
    signing: Inline,
    registry: VectorStoreRegistry,
}

impl Fakes {
    fn new() -> Self {
        let seen = Shared::default();
        Self {
            configs: Configs { seen: seen.clone() },
            filters: Filters,
            executors: Executors { seen: seen.clone() },
            signing: Inline,
            registry: VectorStoreRegistry::default(),
            seen,
        }
    }

    fn source(&self, body: &str) -> Source {
        Source {
            seen: self.seen.clone(),
            answer: (200, HeaderMap::new(), body.as_bytes().to_vec()),
        }
    }

    fn deps<'deps, S>(
        &'deps self,
        source: &'deps S,
        supplied: Option<SuppliedClient<'deps, S::Client>>,
    ) -> SdkDeps<'deps, S>
    where
        S: ClientSource,
    {
        SdkDeps {
            configs: &self.configs,
            filters: &self.filters,
            executors: &self.executors,
            signing: &self.signing,
            registry: Some(&self.registry),
            source,
            supplied,
            settings: settings(),
        }
    }
}

fn search_call<'call>(
    query: &'call SearchQuery,
    optional: &'call Params,
    kwargs: &'call Params,
    entry: Entry,
) -> SearchCall<'call> {
    SearchCall {
        vector_store_id: "vs_a",
        query,
        optional,
        kwargs,
        extra_headers: None,
        extra_body: None,
        timeout: None,
        custom_llm_provider: None,
        executor: None,
        entry,
    }
}

fn create_call<'call>(
    optional: &'call Params,
    kwargs: &'call Params,
    entry: Entry,
) -> CreateCall<'call> {
    CreateCall {
        optional,
        kwargs,
        extra_headers: None,
        extra_body: None,
        timeout: None,
        custom_llm_provider: None,
        entry,
    }
}

#[tokio::test]
async fn create_optional_fields() {
    for entry in [Entry::Sync, Entry::Asynchronous] {
        let fakes = Fakes::new();
        let source = fakes.source(CREATE_RESULT);
        let mut logs = Logs::new("vector_store_create");
        let optional = params(CREATE_OPTIONALS);
        let kwargs = params(r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1"}"#);
        let call = create_call(&optional, &kwargs, entry);
        let deps = fakes.deps(&source, None);
        let result = logs.run_create(&deps, &call).await;
        assert_eq!(
            result.expect("the mock provider answers"),
            json(CREATE_RESULT)
        );
        fakes.seen.with(|seen| {
            assert_eq!(
                seen.bodies.last().expect("the create posted"),
                &json(CREATE_OPTIONALS)
            );
            assert_eq!(seen.create_api_keys, vec![Some(String::from("sk-client"))]);
        });
        assert_eq!(logs.detail("optional_params"), json(CREATE_OPTIONALS));
        assert_eq!(
            logs.detail("custom_llm_provider"),
            Json::Text(String::from("openai"))
        );
        assert_eq!(
            logs.detail("model"),
            Json::Text(String::from("vector-store"))
        );
        let Json::Object(litellm_params) = logs.detail("litellm_params") else {
            panic!("the logged litellm params are an object");
        };
        assert_eq!(litellm_params.get("litellm_call_id"), Some(&Json::Null));
    }
}

#[tokio::test]
async fn search_optional_fields() {
    let fakes = Fakes::new();
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    let optional = params(SEARCH_OPTIONALS);
    let kwargs = params(
        r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1","litellm_call_id":null}"#,
    );
    let query = SearchQuery::Sequence(vec![String::from("one"), String::from("two")]);
    let call = search_call(&query, &optional, &kwargs, Entry::Asynchronous);
    let deps = fakes.deps(&source, None);
    let result = logs.run_search(&deps, &call).await;
    assert_eq!(
        result.expect("the mock provider answers"),
        json(SEARCH_RESULT)
    );
    fakes.seen.with(|seen| {
        assert_eq!(
            seen.search_optionals.last().expect("the transform ran"),
            &params(SEARCH_OPTIONALS)
        );
        assert_eq!(
            seen.search_timeouts,
            vec![Some(Duration::from_secs_f64(6000.0))]
        );
    });
    assert_eq!(
        logs.detail("optional_params"),
        json(
            r#"{"vector_store_id":"vs_a","query":["one","two"],"filters":{"type":"eq","key":"tag","value":"x"},"max_num_results":0,"ranking_options":{"ranker":"auto"},"rewrite_query":false}"#
        )
    );
    let Json::Object(litellm_params) = logs.detail("litellm_params") else {
        panic!("the logged litellm params are an object");
    };
    assert_eq!(litellm_params.get("litellm_call_id"), Some(&Json::Null));
    assert_eq!(
        litellm_params.get("vector_store_id"),
        Some(&Json::Text(String::from("vs_a")))
    );
    assert_eq!(
        litellm_params.get("use_litellm_proxy"),
        Some(&Json::Bool(false))
    );
    assert_eq!(
        logs.detail("custom_llm_provider"),
        Json::Text(String::from("openai"))
    );
    assert_eq!(
        logs.detail("model"),
        Json::Text(String::from("vector-store"))
    );
}

#[tokio::test]
async fn search_timeout_follows_request_timeout() {
    for (given, expected) in [
        (None, Duration::from_secs_f64(6000.0)),
        (Some(0.0), Duration::from_secs_f64(6000.0)),
        (Some(3.0), Duration::from_secs(3)),
    ] {
        let fakes = Fakes::new();
        let source = fakes.source(SEARCH_RESULT);
        let mut logs = Logs::new("vector_store_search");
        let optional = Params::new();
        let kwargs = params(r#"{"api_base":"https://mock.invalid/v1"}"#);
        let query = SearchQuery::Text(String::from("one"));
        let mut call = search_call(&query, &optional, &kwargs, Entry::Sync);
        call.timeout = given;
        let deps = fakes.deps(&source, None);
        logs.run_search(&deps, &call)
            .await
            .expect("the mock provider answers");
        fakes
            .seen
            .with(|seen| assert_eq!(seen.search_timeouts, vec![Some(expected)]));
    }
}

#[tokio::test]
async fn registry_overlays_search_credentials() {
    let mut fakes = Fakes::new();
    let credentials = JsonObject::from([(
        String::from("api_key"),
        Json::Text(String::from("sk-registry")),
    )]);
    fakes.registry = VectorStoreRegistry {
        stores: vec![managed_store(Some("store-credential"))],
        clock: Arc::new(Utc::now),
        credentials: Arc::new(FixedCredentials(credentials)),
    };
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    let query = SearchQuery::Text(String::from("one"));
    let kwargs = params(
        r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1","litellm_call_id":null}"#,
    );
    let optional = params(r#"{"name":"notes"}"#);
    let call = search_call(&query, &optional, &kwargs, Entry::Sync);
    let deps = fakes.deps(&source, None);
    logs.run_search(&deps, &call)
        .await
        .expect("the mock provider answers");
    fakes.seen.with(|seen| {
        assert_eq!(
            seen.search_litellm_params
                .last()
                .expect("the transform ran")
                .get("api_key"),
            Some(&Json::Text(String::from("sk-registry")))
        );
    });
    // create never reads the registry, so the caller's key is the one it uses
    let create_source = fakes.source(CREATE_RESULT);
    let create_optional = params(r#"{"name":"notes"}"#);
    let create_kwargs = params(r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1"}"#);
    let create_invocation = create_call(&create_optional, &create_kwargs, Entry::Sync);
    let create_deps = fakes.deps(&create_source, None);
    logs.run_create(&create_deps, &create_invocation)
        .await
        .expect("the mock provider answers");
    fakes.seen.with(|seen| {
        assert_eq!(
            seen.create_api_keys.last(),
            Some(&Some(String::from("sk-client")))
        );
    });
    // a non-executor fails before the registry, the mocks or any HTTP
    let count = fakes.seen.with(|seen| seen.clients.len());
    let mut invalid = search_call(&query, &optional, &kwargs, Entry::Sync);
    invalid.executor = Some(DirectExecutor::Invalid);
    let error = logs
        .run_search(&deps, &invalid)
        .await
        .expect_err("a non-executor is refused");
    assert_eq!(
        error.to_string(),
        "Invalid direct vector store embedding executor"
    );
    assert_eq!(fakes.seen.with(|seen| seen.clients.len()), count);
}

#[tokio::test]
async fn supplied_executor_reaches_the_embedding_source() {
    let fakes = Fakes::new();
    let source = fakes.source(SEARCH_RESULT);
    let executor = RecordingExecutor {
        _request_kwargs: Params::new(),
    };
    let mut logs = Logs::new("vector_store_search");
    let kwargs = params(r#"{"api_base":"https://mock.invalid/v1"}"#);
    let optional = Params::new();
    let query = SearchQuery::Text(String::from("one"));
    let mut call = search_call(&query, &optional, &kwargs, Entry::Sync);
    call.executor = Some(DirectExecutor::Executor(&executor));
    let deps = fakes.deps(&source, None);
    logs.run_search(&deps, &call)
        .await
        .expect("the mock provider answers");
    fakes
        .seen
        .with(|seen| assert_eq!(seen.executor_supplied, vec![true]));
}

#[tokio::test]
async fn vector_mock_truthiness() {
    let fakes = Fakes::new();
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    let query = SearchQuery::Text(String::from("one"));
    let optional = Params::new();
    let cases = [
        (r#"{"mock_response":"yes"}"#, Some(SAMPLE_SEARCH)),
        (
            r#"{"mock_response":[{"score":0.5,"content":[{"text":"hit","type":"text"}]}]}"#,
            Some(
                r#"{"object":"vector_store.search_results.page","search_query":"sample query","data":[{"score":0.5,"content":[{"text":"hit","type":"text"}]}]}"#,
            ),
        ),
        (r#"{"mock_response":""}"#, None),
        (r#"{"mock_response":[]}"#, None),
        (r#"{"mock_response":{"anything":"goes"}}"#, None),
    ];
    for (mock, expected) in cases {
        let kwargs = params(mock);
        let call = search_call(&query, &optional, &kwargs, Entry::Sync);
        let deps = fakes.deps(&source, None);
        let result = logs
            .run_search(&deps, &call)
            .await
            .expect("the search answers");
        match expected {
            Some(text) => assert_eq!(result, json(text)),
            None => assert_eq!(result, json(SEARCH_RESULT)),
        }
    }
    // an empty create dict proceeds to HTTP, and a nonempty one comes back unchanged
    for (mock, posted) in [
        (
            r#"{"api_base":"https://mock.invalid/v1","mock_response":{}}"#,
            true,
        ),
        (
            r#"{"api_base":"https://mock.invalid/v1","mock_response":{"id":"vs_a","object":"vector_store"}}"#,
            false,
        ),
    ] {
        let kwargs = params(mock);
        let call = create_call(&optional, &kwargs, Entry::Sync);
        let deps = fakes.deps(&source, None);
        let before = fakes.seen.with(|seen| seen.clients.len());
        let result = logs
            .run_create(&deps, &call)
            .await
            .expect("the create answers");
        assert_eq!(result, json(CREATE_RESULT));
        let after = fakes.seen.with(|seen| seen.clients.len());
        assert_eq!(after > before, posted);
    }
    // a null mock takes LiteLLM's sample, and a list replaces the data it carries
    assert_eq!(super::mock::mock_search_response(None), json(SAMPLE_SEARCH));
    assert_eq!(
        super::mock::mock_search_response(Some(&Json::List(Vec::new()))),
        json(SEARCH_RESULT)
    );
    assert_eq!(
        super::mock::mock_create_response(None),
        json(
            r#"{"id":"vs_mock123","object":"vector_store","created_at":1699061776,"name":"Mock Vector Store","bytes":0,"file_counts":{"in_progress":0,"completed":0,"failed":0,"cancelled":0,"total":0},"status":"completed","expires_after":null,"expires_at":null,"last_active_at":null,"metadata":null}"#
        )
    );
}

#[test]
fn vector_provider_resolution() {
    let missing = resolve_provider("search", None).expect("null defaults to openai");
    assert_eq!(missing.provider.to_string(), "openai");
    assert_eq!(missing.api_type, None);
    assert_eq!(missing.tag, VectorStoresConfigTag::OpenAI);
    let empty = resolve_provider("search", Some(""))
        .err()
        .expect("an empty provider is not an enum member");
    assert_eq!(empty.to_string(), " is not a valid LlmProviders");
    let unknown = resolve_provider("create", Some("bogus"))
        .err()
        .expect("an unknown provider is not an enum member");
    assert_eq!(unknown.to_string(), "bogus is not a valid LlmProviders");
    let vertex =
        resolve_provider("search", Some("vertex_ai/rag_api")).expect("the vertex prefix resolves");
    assert_eq!(vertex.api_type, Some(String::from("rag_api")));
    assert_eq!(vertex.tag, VectorStoresConfigTag::Vertex);
    let unsupported = resolve_provider("search", Some("anthropic"))
        .err()
        .expect("anthropic serves no vector store search");
    assert_eq!(
        unsupported.to_string(),
        "Vector store search is not supported for anthropic"
    );
    let unsupported_create = resolve_provider("create", Some("anthropic"))
        .err()
        .expect("anthropic serves no vector store create");
    assert_eq!(
        unsupported_create.to_string(),
        "Vector store create is not supported for anthropic"
    );
    // only a literal true selects the asynchronous handler
    assert_eq!(
        entry_from_flag(&params(r#"{"asearch":true}"#), "asearch"),
        Entry::Asynchronous
    );
    assert_eq!(
        entry_from_flag(&params(r#"{"asearch":1}"#), "asearch"),
        Entry::Sync
    );
    assert_eq!(
        entry_from_flag(&params(r#"{"asearch":"true"}"#), "asearch"),
        Entry::Sync
    );
    assert_eq!(entry_from_flag(&Params::new(), "asearch"), Entry::Sync);
}

#[test]
fn local_errors_map_to_api_connection() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    let mapped = failure(
        &VectorStoreSearchError::Unsupported {
            operation: "create",
            provider: String::from("anthropic"),
        },
        &redactor,
    );
    assert_eq!(
        mapped,
        SdkFailure {
            class: String::from("APIConnectionError"),
            status: Some(500),
            kind: WireField::Null,
            param: WireField::Null,
            code: WireField::Null,
            message: String::from("Vector store create is not supported for anthropic"),
        }
    );
    let invalid = failure(&VectorStoreSearchError::InvalidEmbeddingExecutor, &redactor);
    assert_eq!(invalid.class, "APIConnectionError");
    assert_eq!(invalid.status, Some(500));
    assert_eq!(invalid.kind, WireField::Null);
    let provider = failure(
        &VectorStoreSearchError::Search {
            provider: String::from("openai"),
            source: SearchError::Provider(BaseLlmException {
                status_code: 404,
                message: String::from("missing"),
                headers: ErrorHeaderMap::new(),
                status_code_is_synthesized: false,
            }),
        },
        &redactor,
    );
    assert_eq!(provider.class, "NotFoundError");
    assert_eq!(provider.status, Some(404));
    assert_eq!(provider.kind, WireField::Null);
    assert_eq!(provider.param, WireField::Null);
    assert_eq!(provider.code, WireField::Null);
}

#[tokio::test]
async fn supplied_client_reaches_the_transport() {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let supplied = FakeClient {
        name: String::from("supplied"),
        answer: (200, HeaderMap::new(), SEARCH_RESULT.as_bytes().to_vec()),
        seen: Shared::default(),
    };
    let mut logs = Logs::new("vector_store_search");
    let optional = Params::new();
    let kwargs = params(r#"{"api_base":"https://mock.invalid/v1"}"#);
    let query = SearchQuery::Text(String::from("one"));
    let call = search_call(&query, &optional, &kwargs, Entry::Asynchronous);
    let deps = fakes.deps(
        &source,
        Some(SuppliedClient {
            kind: ClientKind::Asynchronous,
            transport: &supplied,
        }),
    );
    logs.run_search(&deps, &call)
        .await
        .expect("the supplied client answers");
    supplied
        .seen
        .with(|seen| assert_eq!(seen.clients, vec![String::from("supplied")]));
    fakes
        .seen
        .with(|seen| assert_eq!(seen.clients, Vec::<String>::new()));
}

// a create with no optionals sends the five declared fields as nulls
#[tokio::test]
async fn create_sends_the_declared_fields() {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = Params::new();
    let kwargs = params(r#"{"api_base":"https://mock.invalid/v1"}"#);
    let mut headers = BTreeMap::new();
    headers.insert("X-Caller".to_owned(), "yes".to_owned());
    let mut call = create_call(&optional, &kwargs, Entry::Sync);
    call.extra_headers = Some(&headers);
    let deps = fakes.deps(&source, None);
    logs.run_create(&deps, &call)
        .await
        .expect("the mock provider answers");
    fakes.seen.with(|seen| {
        assert_eq!(
            seen.bodies.last().expect("the create posted"),
            &json(
                r#"{"name":null,"file_ids":null,"expires_after":null,"chunking_strategy":null,"metadata":null}"#
            )
        );
        let sent = seen.requests.last().expect("the request was recorded");
        assert_eq!(
            sent.headers
                .get("X-Caller")
                .map(|value| value.to_str().expect("the header is text")),
            Some("yes")
        );
    });
}

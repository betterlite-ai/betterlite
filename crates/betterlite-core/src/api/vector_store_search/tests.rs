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

use self::search::SearchConfigFake;
use super::error::{SdkFailure, VectorStoreSearchError, failure};
use super::executor::{DirectExecutor, EmbeddingExecutorSource, SelectedExecutor};
use super::provider::resolve_provider;
use super::service::{Entry, OptionalParamFilters, SdkDeps, VectorStoreConfigs, entry_from_flag};
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
    BaseLlmException, ClientError, HeaderMap as WireHeaders, UrlFailure,
};
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, SuppliedClient, WriteRequest, WriteTransport,
};
use crate::api::vector_store_registry::store::{VectorStoreCredentials, VectorStoreRegistry};

mod create;
mod search;

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
    fn with<R, F>(&self, read: F) -> R
    where
        F: FnOnce(&mut Seen) -> R,
    {
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
        if !matches!(tag, VectorStoresConfigTag::OpenAI) {
            return None;
        }
        Some(Box::new(CreateConfig {
            seen: self.seen.clone(),
        }))
    }

    fn search_config(&self, tag: VectorStoresConfigTag) -> Option<Arc<dyn SearchConfig>> {
        if !matches!(tag, VectorStoresConfigTag::OpenAI) {
            return None;
        }
        Some(Arc::new(SearchConfigFake {
            seen: self.seen.clone(),
        }))
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
                WireHeaders::new(),
            )
        })
    }

    fn error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: WireHeaders,
    ) -> BaseLlmException {
        exception(message, status_code, headers)
    }
}

fn exception(message: &str, status_code: u16, headers: WireHeaders) -> BaseLlmException {
    BaseLlmException {
        status_code,
        message: message.to_owned(),
        headers,
        status_code_is_synthesized: false,
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
        custom_llm_provider: None,
        vector_store_name: None,
        vector_store_description: None,
        vector_store_metadata: None,
        created_at: None,
        updated_at: None,
        litellm_credential_name: credential_name.map(str::to_owned),
        litellm_params: None,
        team_id: None,
        user_id: None,
        is_config: None,
        extra: BTreeMap::new(),
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
                headers: WireHeaders::new(),
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

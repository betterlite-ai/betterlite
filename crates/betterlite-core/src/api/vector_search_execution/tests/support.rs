use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap as HttpHeaders;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;

use crate::api::vector_search_execution::config::{
    DirectSearchConfig, DirectSearchRequest, EmbeddingEntry, Params, PreparedSearch,
    QueryEmbeddingConfig, QueryEmbeddingExecutor, QuerySearchRequest, SearchConfig, SearchQuery,
    SearchRequest, SignRequest, StoreFailure, VectorStoreLogging, VectorStoreResponse,
};
use crate::api::vector_search_execution::error::{DirectFailure, SearchError, SigningFailure};
use crate::api::vector_search_execution::service::direct::{DirectCall, execute_direct_search};
use crate::api::vector_search_execution::service::signing::{SignFuture, SignInput, SigningSeam};
use crate::api::vector_search_execution::service::{SearchCall, SearchDeps, prepare, send};
use crate::api::vector_store_http_write::error::{BaseLlmException, ClientError, HeaderMap};
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, LogContext, WriteRequest, WriteTransport,
};

#[derive(Clone, Default)]
pub(crate) struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(crate) fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

fn pre_post_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Sink,
    logger: &'env (dyn Fn(&Json) -> Result<(), PrePostError> + Sync),
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
        logger_fn: Some(logger),
        now,
    }
}

// The pre-call every production path runs, with a logger function that records that it happened
pub(crate) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    logger: Box<dyn Fn(&Json) -> Result<(), PrePostError> + Sync>,
    history: Arc<Mutex<Vec<Json>>>,
    pub(crate) state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(crate) fn new(call_type: &str, events: &Events) -> Self {
        let recorder = events.clone();
        let history: Arc<Mutex<Vec<Json>>> = Arc::default();
        let captured = Arc::clone(&history);
        let logger = Box::new(move |details: &Json| -> Result<(), PrePostError> {
            recorder.push("log");
            captured
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(details.clone());
            Ok(())
        });
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
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            redactor,
            sink: Sink,
            logger,
            history,
            state,
            now,
        }
    }

    pub(crate) async fn search<S, G>(
        &mut self,
        deps: &SearchDeps<'_, S>,
        seam: &G,
        mode: ClientKind,
        call: &SearchCall<'_>,
    ) -> Result<VectorStoreSearchResponse, SearchError>
    where
        S: ClientSource,
        S::Client: WriteTransport,
        G: SigningSeam + ?Sized,
    {
        let env = pre_post_env(&self.redactor, &self.sink, self.logger.as_ref(), self.now);
        let prepared = prepare(
            deps,
            seam,
            mode,
            call,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
        .await?;
        send(prepared).await
    }

    pub(crate) async fn direct<C>(
        &mut self,
        config: &C,
        call: &DirectCall<'_>,
        mode: ClientKind,
    ) -> Result<VectorStoreSearchResponse, SearchError>
    where
        C: DirectSearchConfig + ?Sized,
    {
        let env = pre_post_env(&self.redactor, &self.sink, self.logger.as_ref(), self.now);
        execute_direct_search(
            config,
            call,
            mode,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
        .await
    }

    pub(crate) fn logged(&self, name: &str) -> Option<&Json> {
        let Json::Object(details) = self.state.model_call_details.get("additional_args")? else {
            return None;
        };
        details.get(name)
    }

    pub(crate) fn input(&self) -> Option<&Json> {
        self.state.model_call_details.get("input")
    }

    pub(crate) fn detail(&self, name: &str) -> Option<&Json> {
        self.state.model_call_details.get(name)
    }

    pub(crate) fn history(&self) -> Vec<Json> {
        self.history
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: HttpHeaders,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.insert(
            name,
            http::HeaderValue::from_str(value).expect("the header value"),
        );
        self
    }
}

#[derive(Clone)]
pub(crate) struct Wire {
    reply: Reply,
    failure: Arc<Mutex<Option<TransportError>>>,
    requests: Arc<Mutex<Vec<WriteRequest>>>,
    events: Events,
}

impl Wire {
    pub(crate) fn replying(reply: Reply, events: &Events) -> Self {
        Self {
            reply,
            failure: Arc::default(),
            requests: Arc::default(),
            events: events.clone(),
        }
    }

    pub(crate) fn failing(error: TransportError, events: &Events) -> Self {
        let wire = Self::replying(Reply::json(200, "{}"), events);
        *wire.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        wire
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<WriteRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl WriteTransport for Wire {
    fn default_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn send(
        &self,
        request: WriteRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.events.push("send");
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if let Some(error) = failure {
            return ready(Err(error));
        }
        ready(Ok(UpstreamResponse {
            status: self.reply.status,
            headers: self.reply.headers.clone(),
            body: self.reply.body.clone(),
        }))
    }
}

pub(crate) struct Source {
    wire: Wire,
    calls: Arc<Mutex<Vec<Option<bool>>>>,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self {
            wire,
            calls: Arc::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Option<bool>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(&self, ssl_verify: Option<bool>) -> Result<Wire, ClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify);
        Ok(self.wire.clone())
    }
}

#[derive(Default)]
pub(crate) struct Recorded {
    pub(crate) validations: Vec<(OpenAiHeaders, Params)>,
    pub(crate) returned: Vec<OpenAiHeaders>,
    pub(crate) urls: Vec<Params>,
    pub(crate) transforms: Vec<TransformRecord>,
    pub(crate) query_transforms: Vec<TransformRecord>,
    pub(crate) signs: Vec<SignRecord>,
    pub(crate) response_details: Vec<Params>,
    pub(crate) threads: Vec<std::thread::ThreadId>,
}

pub(crate) struct TransformRecord {
    pub(crate) vector_store_id: String,
    pub(crate) query: SearchQuery,
    pub(crate) optional_params: Params,
    pub(crate) api_base: String,
    pub(crate) litellm_params: Params,
    pub(crate) extra_body: Option<Params>,
    pub(crate) executor: bool,
    pub(crate) async_entry: bool,
}

fn transform_record(request: &SearchRequest<'_>, async_entry: bool) -> TransformRecord {
    TransformRecord {
        vector_store_id: request.vector_store_id.to_owned(),
        query: request.query.clone(),
        optional_params: request.optional_params.clone(),
        api_base: request.api_base.to_owned(),
        litellm_params: request.litellm_params.clone(),
        extra_body: request.extra_body.cloned(),
        executor: false,
        async_entry,
    }
}

pub(crate) struct SignRecord {
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) optional_params: Params,
    pub(crate) request_data: Params,
    pub(crate) api_base: String,
}

// A provider whose every member the tests can set and read, standing in for one the caller supplies
pub(crate) struct RecordingConfig {
    base: String,
    url: String,
    body: Params,
    pub(crate) signed_body: Option<Vec<u8>>,
    pub(crate) signs_with_aws: bool,
    pub(crate) query_embedding: bool,
    pub(crate) content_type: Option<&'static str>,
    pub(crate) transform_failure: Option<&'static str>,
    events: Events,
    recorded: Arc<Mutex<Recorded>>,
}

impl RecordingConfig {
    pub(crate) fn new(base: &str, events: &Events) -> Self {
        Self {
            base: base.to_owned(),
            url: format!("{base}/vector_stores/vs_a/search"),
            body: Params::from([(
                "query".to_owned(),
                Json::List(vec![Json::Text("a".to_owned())]),
            )]),
            signed_body: None,
            signs_with_aws: false,
            query_embedding: false,
            content_type: Some("application/json"),
            transform_failure: None,
            events: events.clone(),
            recorded: Arc::default(),
        }
    }

    pub(crate) fn recorder(&self) -> Arc<Mutex<Recorded>> {
        Arc::clone(&self.recorded)
    }

    pub(crate) fn with_signed_body(mut self, body: Option<&[u8]>) -> Self {
        self.signed_body = body.map(<[u8]>::to_vec);
        self
    }

    pub(crate) fn with_url(mut self, url: &str) -> Self {
        self.url = url.to_owned();
        self
    }

    pub(crate) const fn with_content_type(mut self, content_type: Option<&'static str>) -> Self {
        self.content_type = content_type;
        self
    }

    fn recorded(&self) -> MutexGuard<'_, Recorded> {
        self.recorded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn transform_with(
        &self,
        request: SearchRequest<'_>,
        async_entry: bool,
    ) -> Result<PreparedSearch, SearchError> {
        if let Some(field) = self.transform_failure {
            return Err(SearchError::Field {
                field,
                expected: "an integer",
            });
        }
        request.logging.model_call_details.insert(
            "query".to_owned(),
            Json::Text("recorded-by-transform".to_owned()),
        );
        self.recorded()
            .transforms
            .push(transform_record(&request, async_entry));
        Ok((self.url.clone(), self.body.clone()))
    }

    fn query_transform_with(
        &self,
        request: QuerySearchRequest<'_>,
        async_entry: bool,
    ) -> PreparedSearch {
        let mut record = transform_record(&request.search, async_entry);
        record.executor = request.embedding_executor.is_some();
        self.recorded().query_transforms.push(record);
        (self.url.clone(), self.body.clone())
    }
}

#[async_trait]
impl SearchConfig for RecordingConfig {
    fn validate_environment(
        &self,
        mut headers: OpenAiHeaders,
        litellm_params: &Params,
    ) -> Result<OpenAiHeaders, SearchError> {
        self.recorded()
            .validations
            .push((headers.clone(), litellm_params.clone()));
        headers.insert(
            "Authorization".to_owned(),
            SecretString::from("Bearer sk-provider"),
        );
        if let Some(content_type) = self.content_type {
            headers.insert("Content-Type".to_owned(), SecretString::from(content_type));
        }
        self.recorded().returned.push(headers.clone());
        Ok(headers)
    }

    fn complete_url(&self, litellm_params: &Params) -> Result<String, SearchError> {
        self.recorded().urls.push(litellm_params.clone());
        Ok(self.base.clone())
    }

    fn transform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<PreparedSearch, SearchError> {
        self.transform_with(request, false)
    }

    async fn atransform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<PreparedSearch, SearchError> {
        self.transform_with(request, true)
    }

    fn transform_search_response(
        &self,
        response: &VectorStoreResponse,
        logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, SearchError> {
        self.recorded()
            .response_details
            .push(logging.model_call_details.clone());
        serde_json::from_slice(&response.body).map_err(|source| {
            SearchError::Provider(self.error_class(
                &source.to_string(),
                response.status_code,
                HeaderMap::new(),
            ))
        })
    }

    fn sign_request(&self, request: SignRequest<'_>) -> (OpenAiHeaders, Option<Vec<u8>>) {
        self.events.push("sign");
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        self.recorded().threads.push(std::thread::current().id());
        self.recorded().signs.push(SignRecord {
            headers,
            optional_params: request.optional_params.clone(),
            request_data: request.request_data.clone(),
            api_base: request.api_base.to_owned(),
        });
        let mut signed = request.headers.clone();
        signed.insert("X-Signed".to_owned(), SecretString::from("sig-1"));
        (signed, self.signed_body.clone())
    }

    fn signs_with_aws(&self) -> bool {
        self.signs_with_aws
    }

    fn query_embedding_config(&self) -> Option<&dyn QueryEmbeddingConfig> {
        if self.query_embedding {
            Some(self)
        } else {
            None
        }
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

#[async_trait]
impl QueryEmbeddingConfig for RecordingConfig {
    fn transform_query_search(
        &self,
        request: QuerySearchRequest<'_>,
    ) -> Result<PreparedSearch, SearchError> {
        Ok(self.query_transform_with(request, false))
    }

    async fn atransform_query_search(
        &self,
        request: QuerySearchRequest<'_>,
    ) -> Result<PreparedSearch, SearchError> {
        Ok(self.query_transform_with(request, true))
    }
}

// The caller's embedding executor, which a query-embedding transform receives; a search never embeds by itself
pub(crate) struct Embeddings;

#[async_trait]
impl QueryEmbeddingExecutor for Embeddings {
    async fn embed(
        &self,
        _entry: EmbeddingEntry,
        _model: &str,
        _query: &str,
        _configuration: &Params,
    ) -> Result<EmbeddingResponse, StoreFailure> {
        Err(Box::new(std::io::Error::other(
            "the embedding fixture refuses",
        )))
    }
}

// B595's seam over a JoinSet: an AWS-signed config signs on the blocking pool, every other inline
pub(crate) struct ThreadedSeam;

impl SigningSeam for ThreadedSeam {
    fn sign(&self, config: Arc<dyn SearchConfig>, request: SignInput) -> SignFuture<'_> {
        let off_loop = config.signs_with_aws();
        Box::pin(async move {
            if !off_loop {
                return Ok(config.sign_request(request.request()));
            }
            let mut tasks = JoinSet::new();
            tasks.spawn_blocking(move || config.sign_request(request.request()));
            match tasks.join_next().await {
                Some(Ok(signed)) => Ok(signed),
                Some(Err(error)) => Err(SigningFailure {
                    source: Box::new(error),
                }),
                None => Err(SigningFailure {
                    source: Box::new(std::io::Error::other("no signing task ran")),
                }),
            }
        })
    }
}

pub(crate) struct RefusingSeam;

impl SigningSeam for RefusingSeam {
    fn sign(&self, _config: Arc<dyn SearchConfig>, _request: SignInput) -> SignFuture<'_> {
        Box::pin(async {
            Err(SigningFailure {
                source: Box::new(std::io::Error::other("the signing fixture refuses")),
            })
        })
    }
}

pub(crate) struct DirectStore {
    failure: bool,
    observed: Arc<Mutex<DirectObserved>>,
}

#[derive(Default)]
pub(crate) struct DirectObserved {
    pub(crate) ids: Vec<String>,
    pub(crate) queries: Vec<SearchQuery>,
    pub(crate) optional_params: Vec<Params>,
    pub(crate) timeouts: Vec<Option<Duration>>,
    pub(crate) executors: Vec<bool>,
    pub(crate) litellm_params: Vec<Params>,
    pub(crate) async_entries: Vec<bool>,
}

fn record_direct(
    request: &DirectSearchRequest<'_>,
    observed: &mut DirectObserved,
    async_entry: bool,
) {
    observed.ids.push(request.vector_store_id.to_owned());
    observed.queries.push(request.query.clone());
    observed
        .optional_params
        .push(request.optional_params.clone());
    observed.timeouts.push(request.timeout);
    observed
        .executors
        .push(request.embedding_executor.is_some());
    observed.litellm_params.push(request.litellm_params.clone());
    observed.async_entries.push(async_entry);
}

impl DirectStore {
    pub(crate) fn new(failure: bool) -> Self {
        Self {
            failure,
            observed: Arc::default(),
        }
    }

    pub(crate) fn observed(&self) -> MutexGuard<'_, DirectObserved> {
        self.observed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn answer(
        &self,
        request: &DirectSearchRequest<'_>,
        async_entry: bool,
    ) -> Result<VectorStoreSearchResponse, DirectFailure> {
        {
            let mut observed = self.observed.lock().unwrap_or_else(PoisonError::into_inner);
            record_direct(request, &mut observed, async_entry);
        }
        if self.failure {
            return Err(DirectFailure(Box::new(std::io::Error::other(
                "the store refused",
            ))));
        }
        serde_json::from_str(SEARCH_RESPONSE).map_err(|source| DirectFailure(Box::new(source)))
    }
}

#[async_trait]
impl DirectSearchConfig for DirectStore {
    fn execute_search_request(
        &self,
        request: DirectSearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, DirectFailure> {
        self.answer(&request, false)
    }

    async fn aexecute_search_request(
        &self,
        request: DirectSearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, DirectFailure> {
        self.answer(&request, true)
    }
}

// A config that overrides neither async transform, so the trait's sync-to-async defaults run
pub(crate) struct DelegatingConfig {
    inner: RecordingConfig,
}

impl DelegatingConfig {
    pub(crate) fn new() -> Self {
        Self {
            inner: RecordingConfig::new("http://mock.test/v1", &Events::default()),
        }
    }

    pub(crate) const fn with_query_embedding(mut self) -> Self {
        self.inner.query_embedding = true;
        self
    }

    pub(crate) fn recorder(&self) -> Arc<Mutex<Recorded>> {
        self.inner.recorder()
    }
}

impl Default for DelegatingConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchConfig for DelegatingConfig {
    fn validate_environment(
        &self,
        headers: OpenAiHeaders,
        litellm_params: &Params,
    ) -> Result<OpenAiHeaders, SearchError> {
        self.inner.validate_environment(headers, litellm_params)
    }

    fn complete_url(&self, litellm_params: &Params) -> Result<String, SearchError> {
        self.inner.complete_url(litellm_params)
    }

    fn transform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<PreparedSearch, SearchError> {
        self.inner.transform_search_request(request)
    }

    fn transform_search_response(
        &self,
        response: &VectorStoreResponse,
        logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, SearchError> {
        self.inner.transform_search_response(response, logging)
    }

    fn sign_request(&self, request: SignRequest<'_>) -> (OpenAiHeaders, Option<Vec<u8>>) {
        self.inner.sign_request(request)
    }

    fn query_embedding_config(&self) -> Option<&dyn QueryEmbeddingConfig> {
        if self.inner.query_embedding {
            Some(self)
        } else {
            None
        }
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        self.inner.error_class(message, status_code, headers)
    }
}

#[async_trait]
impl QueryEmbeddingConfig for DelegatingConfig {
    fn transform_query_search(
        &self,
        request: QuerySearchRequest<'_>,
    ) -> Result<PreparedSearch, SearchError> {
        self.inner.transform_query_search(request)
    }
}

// A store that overrides only the sync execute, so DirectSearchConfig's async default runs
#[derive(Default)]
pub(crate) struct DelegatingStore {
    observed: Arc<Mutex<DirectObserved>>,
}

impl DelegatingStore {
    pub(crate) fn observed(&self) -> MutexGuard<'_, DirectObserved> {
        self.observed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl DirectSearchConfig for DelegatingStore {
    fn execute_search_request(
        &self,
        request: DirectSearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, DirectFailure> {
        {
            let mut observed = self.observed.lock().unwrap_or_else(PoisonError::into_inner);
            record_direct(&request, &mut observed, false);
        }
        serde_json::from_str(SEARCH_RESPONSE).map_err(|source| DirectFailure(Box::new(source)))
    }
}

pub(crate) const SEARCH_RESPONSE: &str =
    r#"{"object":"vector_store.search_results.page","search_query":"override","data":[]}"#;

pub(crate) fn params(json: &str) -> Params {
    serde_json::from_str(json).expect("the params fixture is valid")
}

pub(crate) fn caller_headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn body_bytes(request: &WriteRequest) -> Option<&[u8]> {
    match &request.body {
        RequestBody::Json(body) => Some(body),
        RequestBody::Bytes { data, .. } => Some(data),
        RequestBody::Empty | RequestBody::Multipart { .. } => None,
    }
}

pub(crate) fn body_json(request: &WriteRequest) -> Json {
    let body = body_bytes(request).expect("the request carries a JSON body");
    serde_json::from_slice(body).expect("the request body is JSON")
}

pub(crate) fn sent_header<'request>(
    request: &'request WriteRequest,
    name: &str,
) -> Option<&'request str> {
    request
        .headers
        .get(name)
        .map(|value| value.to_str().expect("the header is text"))
}

pub(crate) fn proxied_client(limits: TransportLimits) -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::build(base_builder().http1_only(), &policy, limits)
        .expect("the proxied client builds")
}

pub(crate) const IMPATIENT: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(100),
    ..TransportLimits::DEFAULT
};

pub(crate) struct ProxiedSource {
    client: ProxiedClient,
}

impl ProxiedSource {
    pub(crate) fn new(limits: TransportLimits) -> Self {
        Self {
            client: proxied_client(limits),
        }
    }
}

impl ClientSource for ProxiedSource {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}

#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::SystemHubClock;
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::settings::Settings as CountSettings;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService,
};
use betterlite_tokenizer::engine::selection::Selector;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use jiff::Timestamp;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::{Builder, Runtime};
use tokio::time::Instant;
use tokio_util::task::TaskTracker;
use uuid::Uuid;

use crate::cache_lookup::target::CacheProvider;
use crate::cache_lookup::transport::CacheTransport;
use crate::credential_sources::sdk::CredentialClock;
use crate::gemini_call::body::body_request;
use crate::gemini_call::error::GeminiCallError;
use crate::gemini_call::{CallDeps, GeminiCall, GeminiReply, completion};
use crate::gemini_http::call::{Clients, SuppliedClient, VertexLlm};
use crate::gemini_http::post::{
    Chunks, GeminiPost, OpenFuture, OpenedReply, PostFuture, PostReply, PostRequest,
};
use crate::gemini_media::gcs_metadata::{GcsTransport, MetadataFuture};
use crate::gemini_response_parts::projection::CallMode;
use crate::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
};
use crate::token_cache::CredentialCache;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::executor::{CredentialExecutor, LoadedCredential};
use crate::token_cache::probe::AccessToken;
use crate::token_cache::record::CredentialSnapshot;
use crate::token_cache::state::TokenState;
use crate::token_refresh::CredentialRefresh;
use crate::vertex_embedding_call::post::{ClientTimeout, PostError};
use betterlite_core::api::vector_store_http_write::service::ClientKind;

pub(super) const TOKEN: &str = "ya29.vertex-test-token";
pub(super) const PROJECT: &str = "project-p";
pub(super) const LOCATION: &str = "us-central1";
pub(super) const MODEL: &str = "gemini-2.5-flash";
pub(super) const NOW_SECOND: i64 = 1_700_000_000;
pub(super) const MESSAGES: &str = r#"[{"role":"user","content":"hi"}]"#;
// a tool result's remote image: the async transform refuses it, the sync history conversion hands it to the fetcher
pub(super) const REMOTE_TOOL_MEDIA: &str = r#"[{"role":"tool","name":"f","content":[{"type":"image_url","image_url":{"url":"http://example.com/cat"}}]}]"#;
pub(super) const BODY: &str = r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#;
pub(super) const ANSWER: &str = r#"{"candidates":[{"content":{"parts":[{"text":"hello"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5}}"#;
pub(super) const STREAM_CHUNK: &str =
    r#"data: {"responseId":"r1","candidates":[{"content":{"parts":[{"text":"he"}]}}]}"#;
pub(super) const SCHEMA_ERROR: &str = "GenerateContentRequest.tools[0].function_declarations[0].parameters.properties: should be non-empty for OBJECT type";

// the order a call touches its dependencies, recorded by the fixtures that stand in for them
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stage {
    Auth,
    Transform,
    Send,
    Open,
}

#[derive(Default)]
pub(super) struct Order {
    stages: Mutex<Vec<Stage>>,
}

impl Order {
    fn record(&self, stage: Stage) {
        self.stages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(stage);
    }

    pub(super) fn stages(&self) -> Vec<Stage> {
        self.stages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Debug)]
pub(super) enum Scripted<Body> {
    Status {
        status: u16,
        headers: HeaderMap,
        body: Body,
    },
    Timeout,
}

pub(super) type Reply = Scripted<Vec<u8>>;
pub(super) type OpenReply = Scripted<Vec<Vec<u8>>>;

pub(super) fn complete_reply(status: u16, body: &str) -> Reply {
    Reply::Status {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn error_reply(status: u16, pairs: &[(&str, &str)], body: &str) -> Reply {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).expect("a fixture header name"),
            HeaderValue::from_str(value).expect("a fixture header value"),
        );
    }
    Reply::Status {
        status,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn stream_reply(status: u16, pieces: Vec<Vec<u8>>) -> OpenReply {
    OpenReply::Status {
        status,
        headers: HeaderMap::new(),
        body: pieces,
    }
}

pub(super) fn line(text: &str) -> Vec<u8> {
    format!("{text}\n\n").into_bytes()
}

#[derive(Clone, Debug)]
pub(super) struct Recorded {
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
    pub(super) timeout: ClientTimeout,
}

pub(super) struct Recorder {
    order: Arc<Order>,
    posts: Mutex<Vec<Recorded>>,
    opens: Mutex<Vec<Recorded>>,
    post_replies: Mutex<VecDeque<Reply>>,
    open_replies: Mutex<VecDeque<OpenReply>>,
}

impl Recorder {
    fn scripted(order: Arc<Order>, posts: Vec<Reply>, opens: Vec<OpenReply>) -> Self {
        Self {
            order,
            posts: Mutex::new(Vec::new()),
            opens: Mutex::new(Vec::new()),
            post_replies: Mutex::new(VecDeque::from(posts)),
            open_replies: Mutex::new(VecDeque::from(opens)),
        }
    }

    pub(super) fn posts(&self) -> Vec<Recorded> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn opens(&self) -> Vec<Recorded> {
        self.opens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn take<Body>(
        requests: &Mutex<Vec<Recorded>>,
        replies: &Mutex<VecDeque<Body>>,
        request: &PostRequest<'_>,
    ) -> Body {
        requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded(request));
        replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the test scripted a reply")
    }
}

fn recorded(request: &PostRequest<'_>) -> Recorded {
    Recorded {
        url: request.url.to_owned(),
        headers: request
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect(),
        body: request.body.to_vec(),
        timeout: request.timeout,
    }
}

impl GeminiPost for Recorder {
    fn post<'request>(&'request self, request: PostRequest<'request>) -> PostFuture<'request> {
        self.order.record(Stage::Send);
        let reply = Self::take(&self.posts, &self.post_replies, &request);
        Box::pin(async move { complete(reply) })
    }

    fn open<'request>(&'request self, request: PostRequest<'request>) -> OpenFuture<'request> {
        self.order.record(Stage::Open);
        let reply = Self::take(&self.opens, &self.open_replies, &request);
        Box::pin(async move { open(reply) })
    }
}

fn answer<Body>(reply: Scripted<Body>) -> Result<(u16, HeaderMap, Body), PostError> {
    match reply {
        Scripted::Status {
            status,
            headers,
            body,
        } => Ok((status, headers, body)),
        Scripted::Timeout => Err(PostError::Timeout),
    }
}

fn complete(reply: Reply) -> Result<PostReply, PostError> {
    let (status, headers, body) = answer(reply)?;
    Ok(PostReply {
        status,
        headers,
        body,
    })
}

fn open(reply: OpenReply) -> Result<OpenedReply, PostError> {
    let (status, headers, pieces) = answer(reply)?;
    Ok(OpenedReply {
        status,
        headers,
        body: chunks(pieces),
    })
}

fn chunks(pieces: Vec<Vec<u8>>) -> Chunks {
    stream::iter(pieces.into_iter().map(Ok)).boxed()
}

struct FixedClock(SystemTime);

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

struct RecordingExecutor {
    order: Arc<Order>,
    loads: AtomicUsize,
    auth_fails: AtomicBool,
}

impl CredentialExecutor for RecordingExecutor {
    fn load<'borrow>(
        &'borrow self,
        _credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.order.record(Stage::Auth);
        self.loads.fetch_add(1, Ordering::SeqCst);
        if self.auth_fails.load(Ordering::SeqCst) {
            return Box::pin(async { Err(CredentialCacheError::MissingToken) });
        }
        let credential = Arc::new(CredentialSnapshot {
            token: Some(SecretString::from(TOKEN.to_owned())),
            state: Some(TokenState::Fresh),
            expires_at: None,
            expired: None,
            valid: None,
            quota_project_id: None,
            project_id: Some(PROJECT.to_owned()),
        });
        let project = Some(PROJECT.to_owned());
        Box::pin(async move {
            Ok(LoadedCredential {
                credential: Some(credential),
                project,
            })
        })
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(async { Err(CredentialCacheError::MissingToken) })
    }
}

struct RecordingLookup {
    order: Arc<Order>,
}

impl ModelCapabilityLookup for RecordingLookup {
    fn supports(
        &self,
        _kind: CapabilityKind,
        _model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        self.order.record(Stage::Transform);
        Ok(true)
    }
}

#[derive(Default)]
struct NoFetch {
    calls: AtomicUsize,
}

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, _url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ready(Err(ImageFetchError::Rejected { url: String::new() }))
    }

    fn convert_urls(
        &self,
        _urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ready(Err(ImageFetchError::Rejected { url: String::new() }))
    }
}

#[derive(Default)]
struct NoTransport {
    calls: AtomicUsize,
}

impl CacheTransport for NoTransport {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ready(Err(TransportError::Closed))
    }
}

impl GcsTransport for NoTransport {
    fn get(&self, _request: OutboundRequest) -> MetadataFuture<'_> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(ready(Err(TransportError::Closed)))
    }
}

struct SilentLogger;

impl LoggerSink for SilentLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        false
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, _payload: &Payload) -> bool {
        false
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

type Hooks = VerboseHooks<SilentLogger, SilentConsole>;

fn hooks() -> Hooks {
    VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redactor compiles"),
        ),
    }
}

struct InlineCounter;

#[async_trait]
impl BlockingCounter for InlineCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

fn hub_settings() -> HubSettings {
    HubSettings {
        endpoint: "http://127.0.0.1:1".to_owned(),
        offline: OfflineSetting::Offline,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: PathBuf::new(),
        hub_cache: PathBuf::new(),
        token_path: PathBuf::new(),
        stored_tokens_path: PathBuf::new(),
        transfer: TransferSetting::Standard,
    }
}

fn counter(catalog: &Arc<ModelCatalog>) -> Counter {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("a proxy policy"),
    );
    let client = Arc::new(ProxiedClient::outbound(&policy).expect("a fixture client"));
    let settings = CountSettings::default();
    let service = Arc::new(CountingService::new(
        4,
        Arc::new(InlineCounter),
        settings.encode_limits().expect("the encode limits"),
    ));
    let fetcher = Arc::new(TiktokenFetcher::new(
        Arc::clone(&client),
        None,
        Arc::clone(&service),
    ));
    let hub = hub_settings();
    let clock = Arc::new(SystemHubClock::default());
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let tokens = Arc::new(HubTokenResolver::new(
        hub.clone(),
        OidcSettings::default(),
        Arc::clone(&client),
        reader,
        clock,
    ));
    let downloader = Arc::new(HubDownloader::new(
        Arc::new(hub),
        tokens,
        Arc::new(Instant::now),
    ));
    let selector = Arc::new(Selector::new(
        fetcher,
        downloader,
        Arc::clone(catalog),
        Arc::clone(&service),
        true,
    ));
    Counter::new(selector, service, Arc::clone(catalog), settings)
}

#[derive(Clone, Copy)]
pub(super) struct Setup {
    pub(super) acompletion: bool,
    pub(super) stream: Option<bool>,
    pub(super) timeout: Option<Duration>,
}

// what a test changes about the default call: Vertex in us-central1 with no key, headers or flags
#[derive(Default)]
pub(super) struct Variation {
    pub(super) provider: Option<VertexCapabilityProvider>,
    pub(super) location: Option<&'static str>,
    pub(super) api_base: Option<&'static str>,
    pub(super) gemini_api_key: Option<SecretString>,
    pub(super) extra_headers: BTreeMap<String, String>,
    pub(super) use_psc_endpoint_format: Option<bool>,
}

pub(super) struct Fixture {
    pub(super) runtime: Runtime,
    pub(super) order: Arc<Order>,
    pub(super) recorder: Arc<Recorder>,
    pub(super) caller: Option<Arc<Recorder>>,
    executor: Arc<RecordingExecutor>,
    llm: VertexLlm,
    clients: Clients,
    catalog: Arc<ModelCatalog>,
    counter: Counter,
    hooks: Hooks,
    fetcher: NoFetch,
    transport: NoTransport,
    lookup: RecordingLookup,
    messages: Vec<Message>,
    litellm_params: Object,
}

// the two client slots a call can reach, with the caller's recorder kept for the assertions on it
fn clients(
    recorder: &Arc<Recorder>,
    supplied: Option<(ClientKind, Arc<Recorder>)>,
) -> (Clients, Option<Arc<Recorder>>) {
    let pooled: Arc<dyn GeminiPost> = Arc::<Recorder>::clone(recorder);
    let (supplied_client, caller) = match supplied {
        Some((kind, transport)) => (
            Some(SuppliedClient {
                kind,
                transport: Arc::<Recorder>::clone(&transport),
            }),
            Some(transport),
        ),
        None => (None, None),
    };
    (
        Clients {
            supplied: supplied_client,
            pooled,
        },
        caller,
    )
}

impl Fixture {
    fn new(
        order: Arc<Order>,
        recorder: Arc<Recorder>,
        supplied: Option<(ClientKind, Arc<Recorder>)>,
        auth_fails: bool,
    ) -> Self {
        let executor = Arc::new(RecordingExecutor {
            order: Arc::clone(&order),
            loads: AtomicUsize::new(0),
            auth_fails: AtomicBool::new(auth_fails),
        });
        let credential_executor: Arc<dyn CredentialExecutor> =
            Arc::<RecordingExecutor>::clone(&executor);
        let cache = Arc::new(CredentialCache::new(
            Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
            credential_executor,
        ));
        let refresh = Arc::new(CredentialRefresh::new(
            Arc::clone(&cache),
            TaskTracker::new(),
        ));
        let (clients, caller) = clients(&recorder, supplied);
        let catalog = BundledCatalog::default().get();
        Self {
            runtime: Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a fixture runtime"),
            llm: VertexLlm { cache, refresh },
            clients,
            caller,
            executor,
            counter: counter(&catalog),
            catalog,
            hooks: hooks(),
            fetcher: NoFetch::default(),
            transport: NoTransport::default(),
            lookup: RecordingLookup {
                order: Arc::clone(&order),
            },
            messages: serde_json::from_str(MESSAGES).expect("fixture messages"),
            litellm_params: Object::new(),
            order,
            recorder,
        }
    }

    pub(super) fn loads(&self) -> usize {
        self.executor.loads.load(Ordering::SeqCst)
    }

    pub(super) fn fetches(&self) -> usize {
        self.fetcher.calls.load(Ordering::SeqCst)
    }

    pub(super) fn with_messages(mut self, messages: &str) -> Self {
        self.messages = serde_json::from_str(messages).expect("fixture messages");
        self
    }

    pub(super) fn cache_calls(&self) -> usize {
        self.transport.calls.load(Ordering::SeqCst)
    }

    fn call<'call>(
        &'call self,
        optional_params: &'call Object,
        setup: Setup,
        variation: &'call Variation,
    ) -> GeminiCall<'call> {
        GeminiCall {
            provider: variation
                .provider
                .unwrap_or(VertexCapabilityProvider::VertexAi),
            model: MODEL,
            messages: &self.messages,
            optional_params,
            litellm_params: &self.litellm_params,
            api_base: variation.api_base,
            gemini_api_key: variation.gemini_api_key.as_ref(),
            project: Some(PROJECT),
            location: variation.location.or(Some(LOCATION)),
            credentials: None,
            extra_headers: &variation.extra_headers,
            timeout: setup.timeout,
            acompletion: setup.acompletion,
            modify_params: false,
            now: Timestamp::from_second(NOW_SECOND).expect("fixture clock"),
            call_mode: CallMode::Tools,
        }
    }

    fn deps(&self) -> CallDeps<'_, NoTransport, NoFetch, SilentLogger, SilentConsole> {
        CallDeps {
            catalog: self.catalog.as_ref(),
            llm: &self.llm,
            clients: &self.clients,
            capabilities: &self.lookup,
            cache: &self.transport,
            fetcher: &self.fetcher,
            gcs: &self.transport,
            counter: &self.counter,
            hooks: &self.hooks,
            schema_template: None,
        }
    }

    pub(super) fn run(&self, setup: Setup) -> Result<GeminiReply, GeminiCallError> {
        self.run_with(setup, &Variation::default())
    }

    pub(super) fn run_with(
        &self,
        setup: Setup,
        variation: &Variation,
    ) -> Result<GeminiReply, GeminiCallError> {
        let optional_params = optional_params(setup, variation);
        let call = self.call(&optional_params, setup, variation);
        let deps = self.deps();
        self.runtime
            .block_on(completion(&call, &deps, Box::new(fixed_id)))
    }

    pub(super) fn cache_deadline(&self, setup: Setup) -> Duration {
        let variation = Variation::default();
        let optional_params = optional_params(setup, &variation);
        let call = self.call(&optional_params, setup, &variation);
        let deps = self.deps();
        let access = AccessToken {
            token: SecretString::from(TOKEN),
            project: PROJECT.to_owned(),
        };
        body_request(&call, &optional_params, &deps, &access)
            .cache
            .deadline
    }

    pub(super) fn cache_provider(&self, provider: VertexCapabilityProvider) -> CacheProvider {
        let variation = Variation {
            provider: Some(provider),
            location: None,
            api_base: None,
            gemini_api_key: None,
            extra_headers: BTreeMap::new(),
            use_psc_endpoint_format: None,
        };
        let setup = Setup {
            acompletion: false,
            stream: None,
            timeout: None,
        };
        let optional_params = optional_params(setup, &variation);
        let call = self.call(&optional_params, setup, &variation);
        let deps = self.deps();
        let access = AccessToken {
            token: SecretString::from(TOKEN),
            project: PROJECT.to_owned(),
        };
        body_request(&call, &optional_params, &deps, &access)
            .cache
            .provider
    }
}

fn optional_params(setup: Setup, variation: &Variation) -> Object {
    let mut optional_params = Object::new();
    if let Some(stream) = setup.stream {
        optional_params.insert("stream".to_owned(), Json::Bool(stream));
    }
    if let Some(flag) = variation.use_psc_endpoint_format {
        optional_params.insert("use_psc_endpoint_format".to_owned(), Json::Bool(flag));
    }
    optional_params
}

fn fixed_id() -> Uuid {
    Uuid::nil()
}

pub(super) fn pooled_fixture(posts: Vec<Reply>, opens: Vec<OpenReply>) -> Fixture {
    let order = Arc::new(Order::default());
    let recorder = Arc::new(Recorder::scripted(Arc::clone(&order), posts, opens));
    Fixture::new(order, recorder, None, false)
}

// a credential cache that refuses every load, so a call never reaches the endpoint
pub(super) fn auth_failure_fixture() -> Fixture {
    let order = Arc::new(Order::default());
    let recorder = Arc::new(Recorder::scripted(
        Arc::clone(&order),
        Vec::new(),
        Vec::new(),
    ));
    Fixture::new(order, recorder, None, true)
}

pub(super) fn supplied_fixture(
    kind: ClientKind,
    caller: (Vec<Reply>, Vec<OpenReply>),
    pooled: (Vec<Reply>, Vec<OpenReply>),
) -> Fixture {
    let order = Arc::new(Order::default());
    let caller_recorder = Arc::new(Recorder::scripted(Arc::clone(&order), caller.0, caller.1));
    let pooled_recorder = Arc::new(Recorder::scripted(Arc::clone(&order), pooled.0, pooled.1));
    Fixture::new(order, pooled_recorder, Some((kind, caller_recorder)), false)
}

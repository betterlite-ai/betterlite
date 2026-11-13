use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, pending, ready};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::cache_key::derive::get_cache_key;
use betterlite_cache::cache_key::request::{CacheKeyRequest, KeyParam};
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::cache_key::value::KeyValue;
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::SystemHubClock;
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
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
use http::Method;
use http::header::HeaderMap;
use jiff::Timestamp;
use secrecy::SecretString;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::cache_create::error::CacheCreateError;
use crate::cache_create::orchestration::{
    CacheCreateOptions, CacheCreateOutcome, CacheCreateRequest, check_and_create_cache,
};
use crate::cache_lookup::target::CacheProvider;
use crate::cache_lookup::transport::CacheTransport;
use crate::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup,
};

pub(crate) const NOW_SECOND: i64 = 1_794_441_600;

pub(crate) struct SilentLogger;

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

pub(crate) struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

pub(crate) type Hooks = VerboseHooks<SilentLogger, SilentConsole>;

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

pub(crate) struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

pub(crate) struct FixtureLookup {
    pub supported: bool,
}

impl ModelCapabilityLookup for FixtureLookup {
    fn supports(
        &self,
        kind: CapabilityKind,
        _model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        match kind {
            CapabilityKind::SystemMessages => Ok(self.supported),
            CapabilityKind::ResponseSchema => Ok(false),
        }
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

fn counter() -> Counter {
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
    let catalog = BundledCatalog::default().get();
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
        Arc::clone(&fetcher),
        downloader,
        Arc::clone(&catalog),
        Arc::clone(&service),
        true,
    ));
    Counter::new(selector, fetcher, service, catalog, settings)
}

pub(crate) struct Harness {
    pub model: String,
    pub provider: CacheProvider,
    pub messages: Vec<Message>,
    pub api_base: Option<String>,
    pub project: Option<String>,
    pub location: Option<String>,
    pub auth_token: Option<SecretString>,
    pub gemini_api_key: Option<SecretString>,
    pub extra_headers: BTreeMap<String, String>,
    pub deadline: Duration,
    pub cached_content: Option<String>,
    pub disable_transform: bool,
    pub now: Timestamp,
    pub minimum_token_override: Option<u64>,
    pub counter: Counter,
    pub catalog: Arc<ModelCatalog>,
    pub capabilities: FixtureLookup,
    pub fetcher: NoFetch,
    pub hooks: Hooks,
}

impl Harness {
    pub(crate) fn new(model: &str, messages: &str) -> Self {
        Self {
            model: model.to_owned(),
            provider: CacheProvider::Vertex,
            messages: serde_json::from_str(messages).expect("fixture messages"),
            api_base: None,
            project: Some("proj".to_owned()),
            location: Some("us-central1".to_owned()),
            auth_token: Some(SecretString::from("token-abc")),
            gemini_api_key: None,
            extra_headers: BTreeMap::new(),
            deadline: Duration::from_secs(5),
            cached_content: None,
            disable_transform: false,
            now: Timestamp::from_second(NOW_SECOND).expect("fixture clock"),
            minimum_token_override: None,
            counter: counter(),
            catalog: BundledCatalog::default().get(),
            capabilities: FixtureLookup { supported: true },
            fetcher: NoFetch,
            hooks: hooks(),
        }
    }

    pub(crate) async fn run<T>(
        &self,
        transport: &T,
        options: &mut CacheCreateOptions,
    ) -> Result<CacheCreateOutcome, CacheCreateError>
    where
        T: CacheTransport,
    {
        let request = CacheCreateRequest {
            provider: self.provider,
            model: &self.model,
            messages: &self.messages,
            api_base: self.api_base.as_deref(),
            project: self.project.as_deref(),
            location: self.location.as_deref(),
            auth_token: self.auth_token.as_ref(),
            gemini_api_key: self.gemini_api_key.as_ref(),
            extra_headers: &self.extra_headers,
            deadline: self.deadline,
            cached_content: self.cached_content.as_deref(),
            disable_transform: self.disable_transform,
            now: self.now,
            minimum_token_override: self.minimum_token_override,
            counter: &self.counter,
            catalog: &self.catalog,
            capabilities: &self.capabilities,
            fetcher: &self.fetcher,
        };
        check_and_create_cache(transport, &request, options, &self.hooks).await
    }
}

pub(crate) enum Reply {
    Ok { status: u16, body: Vec<u8> },
    Failure(TransportError),
}

#[derive(Clone, Debug)]
pub(crate) struct Recorded {
    pub method: Method,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

pub(crate) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Recorded>>,
}

impl ScriptedTransport {
    pub(crate) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().expect("fixture requests").clone()
    }

    pub(crate) fn methods(&self) -> Vec<Method> {
        self.requests()
            .into_iter()
            .map(|record| record.method)
            .collect()
    }
}

impl CacheTransport for ScriptedTransport {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let encoded = match request.body {
            RequestBody::Empty | RequestBody::Multipart { .. } => Vec::new(),
            RequestBody::Json(bytes) => bytes,
            RequestBody::Bytes { data, .. } => data,
        };
        self.requests
            .lock()
            .expect("fixture requests")
            .push(Recorded {
                method: request.method,
                url: request.url.to_string(),
                headers: request.headers,
                body: encoded,
            });
        let reply = self
            .replies
            .lock()
            .expect("fixture replies")
            .pop_front()
            .expect("a scripted reply");
        ready(match reply {
            Reply::Ok {
                status,
                body: response_body,
            } => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: response_body,
            }),
            Reply::Failure(source) => Err(source),
        })
    }
}

pub(crate) struct HangingTransport {
    dropped: Arc<AtomicBool>,
    requests: AtomicUsize,
}

impl Default for HangingTransport {
    fn default() -> Self {
        Self {
            dropped: Arc::default(),
            requests: AtomicUsize::new(0),
        }
    }
}

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl HangingTransport {
    pub(crate) fn was_dropped(&self) -> bool {
        self.dropped.load(Ordering::SeqCst)
    }

    pub(crate) fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl CacheTransport for HangingTransport {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.requests.fetch_add(1, Ordering::SeqCst);
        let dropped = Arc::clone(&self.dropped);
        async move {
            let _guard = DropFlag(dropped);
            pending::<Result<UpstreamResponse, TransportError>>().await
        }
    }
}

pub(crate) struct StallingServer {
    pub base: String,
    pub seen: Arc<Mutex<Vec<String>>>,
    pub _held: Arc<Mutex<Vec<TcpStream>>>,
    pub _tasks: JoinSet<()>,
}

// answers the cached-content list with an empty page and leaves the create request waiting for a reply
pub(crate) async fn stalling_server() -> StallingServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a local listener");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("the bound address")
    );
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let held: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_stalls(listener, Arc::clone(&seen), Arc::clone(&held)));
    StallingServer {
        base,
        seen,
        _held: held,
        _tasks: tasks,
    }
}

async fn serve_stalls(
    listener: TcpListener,
    seen: Arc<Mutex<Vec<String>>>,
    held: Arc<Mutex<Vec<TcpStream>>>,
) {
    loop {
        let Ok((mut stream, _peer)) = listener.accept().await else {
            return;
        };
        let Some(line) = request_line(&mut stream).await else {
            continue;
        };
        seen.lock().expect("fixture requests").push(line.clone());
        if line.starts_with("GET ") {
            let body = r#"{"cachedContents":[]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _written = stream.write_all(response.as_bytes()).await;
        } else {
            held.lock().expect("held streams").push(stream);
        }
    }
}

async fn request_line(stream: &mut TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await.ok()?;
    Some(line.trim_end().to_owned())
}

pub(crate) fn page(body: &str) -> Reply {
    Reply::Ok {
        status: 200,
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn status(status: u16, body: &str) -> Reply {
    Reply::Ok {
        status,
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn list_page(display_name: &str, name: &str) -> Reply {
    page(&format!(
        r#"{{"cachedContents":[{{"displayName":"{display_name}","name":"{name}"}}]}}"#
    ))
}

pub(crate) fn empty_list_page() -> Reply {
    page(r#"{"cachedContents":[]}"#)
}

pub(crate) fn created_cache(name: &str) -> Reply {
    page(&format!(r#"{{"name":"{name}"}}"#))
}

fn key_value<T>(value: &T) -> KeyValue
where
    T: serde::Serialize + ?Sized,
{
    let encoded = serde_json::to_vec(value).expect("fixture key material");
    serde_json::from_slice(&encoded).expect("fixture key material parses")
}

fn local_key_settings() -> CacheKeySettings {
    CacheKeySettings {
        cache_type: LiteLlmCacheType::Local,
        mode: CacheMode::DefaultOn,
        namespace: None,
        semantic_cache_scope: SemanticCacheScope::Key,
        include_provider_params: false,
    }
}

pub(crate) fn expected_cache_key(
    messages: &[Message],
    tools: Option<&[Json]>,
    tool_choice: Option<&Json>,
    model: &str,
) -> String {
    let mut request = CacheKeyRequest {
        params: vec![
            KeyParam {
                name: "messages".to_owned(),
                value: key_value(messages),
            },
            KeyParam {
                name: "tools".to_owned(),
                value: key_value(&tools),
            },
            KeyParam {
                name: "tool_choice".to_owned(),
                value: key_value(&tool_choice),
            },
            KeyParam {
                name: "model".to_owned(),
                value: KeyValue::String(model.to_owned()),
            },
        ],
        metadata: None,
        litellm_metadata: None,
        litellm_params: None,
        cache_control: None,
    };
    get_cache_key(&mut request, &local_key_settings(), &hooks()).expect("the fixture key derives")
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::SystemHubClock;
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::catalog::BundledCatalog;
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
use tokio::time::Instant;

use crate::cache_lookup::target::CacheProvider;
use crate::cache_lookup::transport::CacheTransport;
use crate::gemini_media::gcs_metadata::tests::parse_object;
use crate::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
};

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

pub(crate) fn hooks() -> Hooks {
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

pub(crate) fn counter() -> Counter {
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
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
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

#[derive(Clone, Copy)]
pub(crate) struct Supports {
    pub(crate) system_message: bool,
    pub(crate) response_schema: bool,
}

pub(crate) struct FixtureLookup {
    pub(crate) supports: Supports,
}

impl ModelCapabilityLookup for FixtureLookup {
    fn supports(
        &self,
        kind: CapabilityKind,
        _model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        Ok(match kind {
            CapabilityKind::SystemMessages => self.supports.system_message,
            CapabilityKind::ResponseSchema => self.supports.response_schema,
        })
    }
}

#[derive(Default)]
pub(crate) struct MapFetch {
    urls: BTreeMap<String, String>,
    converted: Mutex<Vec<String>>,
    batches: Mutex<Vec<Vec<String>>>,
}

impl MapFetch {
    pub(crate) fn with_urls(&mut self, urls: &[(&str, &str)]) {
        self.urls = urls
            .iter()
            .map(|(url, data_url)| ((*url).to_owned(), (*data_url).to_owned()))
            .collect();
    }

    fn lookup(&self, url: &str) -> Result<String, ImageFetchError> {
        self.urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Attempts {
                url: url.to_owned(),
            })
    }

    pub(crate) fn calls(&self) -> usize {
        self.converted()
            .len()
            .saturating_add(self.batches.lock().expect("fixture fetcher").len())
    }

    pub(crate) fn converted(&self) -> Vec<String> {
        self.converted.lock().expect("fixture fetcher").clone()
    }

    pub(crate) fn batches(&self) -> Vec<Vec<String>> {
        self.batches.lock().expect("fixture fetcher").clone()
    }
}

impl RemoteMediaFetcher for MapFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.converted
            .lock()
            .expect("fixture fetcher")
            .push(url.to_owned());
        ready(self.lookup(url))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        self.batches
            .lock()
            .expect("fixture fetcher")
            .push(urls.to_vec());
        let values: Result<Vec<String>, ImageFetchError> =
            urls.iter().map(|url| self.lookup(url)).collect();
        ready(values)
    }
}

#[derive(Default)]
pub(crate) struct NoTransport {
    calls: AtomicUsize,
}

impl NoTransport {
    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
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

pub(crate) struct RequestInput {
    pub(crate) provider: VertexCapabilityProvider,
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    pub(crate) optional_params: Object,
    pub(crate) litellm_params: Object,
    pub(crate) modify_params: bool,
    pub(crate) supports: Supports,
}

pub(crate) fn parse_messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).expect("fixture messages")
}

pub(crate) fn parse_json(source: &str) -> Json {
    serde_json::from_str(source).expect("fixture json")
}

pub(crate) const fn cache_provider(provider: VertexCapabilityProvider) -> CacheProvider {
    match provider {
        VertexCapabilityProvider::VertexAi => CacheProvider::Vertex,
        VertexCapabilityProvider::VertexAiBeta => CacheProvider::VertexBeta,
        VertexCapabilityProvider::Gemini => CacheProvider::Gemini,
    }
}

pub(crate) fn vertex_input(model: &str, messages: &str, optional_params: &str) -> RequestInput {
    RequestInput {
        provider: VertexCapabilityProvider::VertexAi,
        model: model.to_owned(),
        messages: parse_messages(messages),
        optional_params: parse_object(optional_params),
        litellm_params: Object::new(),
        modify_params: false,
        supports: Supports {
            system_message: true,
            response_schema: true,
        },
    }
}

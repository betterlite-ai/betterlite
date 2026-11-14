use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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
use betterlite_prompts::arguments::json::Object;
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
use betterlite_vertex::cache_lookup::transport::CacheTransport;
use betterlite_vertex::gemini_request::error::{BodyError, RequestError};
use betterlite_vertex::gemini_request::orchestration::{CallRequest, transform_request_body};
use betterlite_vertex::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
};
use jiff::Timestamp;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tokio::time::Instant;

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

const NOW_SECOND: i64 = 1_794_441_600;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) cases: Vec<Case>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Supports {
    system_message: bool,
    response_schema: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    model: String,
    provider: ProviderName,
    // the case states what the model catalog answers, so the comparison fixes the capability input
    supports: Supports,
    #[serde(default)]
    modify_params: bool,
    messages: Vec<Message>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    litellm_params: Object,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProviderName {
    VertexAi,
    VertexAiBeta,
    Gemini,
}

impl ProviderName {
    const fn capability_provider(self) -> VertexCapabilityProvider {
        match self {
            Self::VertexAi => VertexCapabilityProvider::VertexAi,
            Self::VertexAiBeta => VertexCapabilityProvider::VertexAiBeta,
            Self::Gemini => VertexCapabilityProvider::Gemini,
        }
    }
}

struct FixtureLookup {
    supports: Supports,
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

struct NoFetch;

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

struct NoTransport;

impl CacheTransport for NoTransport {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Err(TransportError::Closed))
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

fn counter() -> Result<Counter, Box<HelperError>> {
    let policy =
        Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).or_config_error()?);
    let client = Arc::new(ProxiedClient::outbound(&policy).or_config_error()?);
    let settings = CountSettings::default();
    let service = Arc::new(CountingService::new(
        4,
        Arc::new(InlineCounter),
        settings.encode_limits().or_config_error()?,
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
    Ok(Counter::new(selector, fetcher, service, catalog, settings))
}

fn hooks() -> Result<Hooks, Box<HelperError>> {
    Ok(VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?),
    })
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(runtime_error)?;
    let mut results = Vec::with_capacity(input.cases.len());
    for case in &input.cases {
        results.push(case_value(case, &runtime)?);
    }
    Ok(json!({ "results": results }))
}

fn runtime_error(error: std::io::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "RuntimeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn case_value(case: &Case, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let auth_token = SecretString::from("token-abc");
    let extra_headers = BTreeMap::new();
    let lookup = FixtureLookup {
        supports: case.supports,
    };
    let token_counter = counter()?;
    let catalog = BundledCatalog::default().get();
    let verbose = hooks()?;
    let fetcher = NoFetch;
    let transport = NoTransport;
    let request = CallRequest {
        provider: case.provider.capability_provider(),
        model: &case.model,
        messages: &case.messages,
        optional_params: &case.optional_params,
        litellm_params: &case.litellm_params,
        modify_params: case.modify_params,
        schema_template: None,
        api_base: None,
        project: Some("project"),
        location: Some("us-central1"),
        auth_token: Some(&auth_token),
        gemini_api_key: None,
        extra_headers: &extra_headers,
        deadline: Duration::from_secs(5),
        disable_transform: false,
        now: Timestamp::from_second(NOW_SECOND).or_value_error()?,
        minimum_token_override: None,
        counter: &token_counter,
        catalog: catalog.as_ref(),
        capabilities: &lookup,
        fetcher: &fetcher,
    };
    Ok(
        match runtime.block_on(transform_request_body(&transport, &request, &verbose)) {
            Ok(transformed) => json!({
                "body": transformed.body,
                "litellm_params": transformed.litellm_params,
            }),
            Err(error) => json!({ "error": error_value(&error) }),
        },
    )
}

fn error_value(error: &RequestError) -> Value {
    match error {
        RequestError::Body(BodyError::History(history)) => json!({
            "status": history.status(),
            "type": history.error_type(),
            "param": history.param(),
            "code": history.code(),
        }),
        RequestError::Cache(source) => json!({
            "status": source.status_code(),
            "type": null,
            "param": null,
            "code": null,
        }),
        RequestError::Body(BodyError::Field { field, expected }) => json!({
            "status": 400,
            "type": null,
            "param": field,
            "code": "400",
            "message": format!("`{field}` must be {expected}"),
        }),
        other => json!({
            "status": 400,
            "type": null,
            "param": null,
            "code": "400",
            "message": other.to_string(),
        }),
    }
}

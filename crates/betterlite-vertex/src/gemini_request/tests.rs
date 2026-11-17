#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::error::MediaError;
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
use jiff::Timestamp;
use rstest::rstest;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};
use tokio::time::Instant;

use crate::cache_create::orchestration::CacheCreateRequest;
use crate::cache_lookup::target::CacheProvider;
use crate::cache_lookup::transport::CacheTransport;
use crate::gemini_request::body::Transformed;
use crate::gemini_request::error::RequestError;
use crate::gemini_request::orchestration::CallRequest;
use crate::gemini_request::orchestration::async_media::transform_request_body_async;
use crate::gemini_request::orchestration::transform_request_body;
use crate::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
};

const NOW_SECOND: i64 = 1_794_441_600;
const USER_MESSAGES: &str = r#"[{"role":"user","content":"hi"}]"#;
const SYSTEM_USER_MESSAGES: &str =
    r#"[{"role":"system","content":"rules"},{"role":"user","content":"hi"}]"#;
const USER_CONTENTS: &str = r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#;
const IMAGE_MESSAGES: &str = r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"data:image/png;base64,YWJj","detail":"high"}}]}]"#;
const FETCHED: &str = "data:image/png;base64,YWJj";
const REMOTE_HTTP: &str = "http://example.com/cat";
const REMOTE_HTTP_PNG: &str = "http://example.com/cat.png";
const REMOTE_HTTPS: &str = "https://example.com/cat";
const REMOTE_HTTPS_PNG: &str = "https://example.com/cat.png";
const REMOTE_A: &str = "https://example.com/a";
const REMOTE_B: &str = "https://example.com/b";
const REMOTE_PDF: &str = "https://example.com/doc.pdf";
const INLINE_CAT_BODY: &str = r#"{"contents":[{"role":"user","parts":[{"text":"what"},{"inline_data":{"data":"YWJj","mime_type":"image/png"}}]}]}"#;
const ASSISTANT_TEXT_BODY: &str = r#"{"contents":[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"}]},{"role":"user","parts":[{"text":"next"}]}]}"#;
const ASSISTANT_JPEG_BODY: &str = r#"{"contents":[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"},{"inline_data":{"data":"YWJj","mime_type":"image/jpeg"}}]},{"role":"user","parts":[{"text":"next"}]}]}"#;
const STUDIO_FILE_MESSAGES: &str = r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"https://example.com/doc.pdf","format":"application/pdf"}}]}]"#;

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
struct Supports {
    system_message: bool,
    response_schema: bool,
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

#[derive(Default)]
struct MapFetch {
    urls: BTreeMap<String, String>,
    converted: Mutex<Vec<String>>,
    batches: Mutex<Vec<Vec<String>>>,
}

impl MapFetch {
    fn with_urls(&mut self, urls: &[(&str, &str)]) {
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

    fn calls(&self) -> usize {
        self.converted()
            .len()
            .saturating_add(self.batches.lock().expect("fixture fetcher").len())
    }

    fn converted(&self) -> Vec<String> {
        self.converted.lock().expect("fixture fetcher").clone()
    }

    fn batches(&self) -> Vec<Vec<String>> {
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
struct NoTransport {
    calls: AtomicUsize,
}

impl NoTransport {
    fn calls(&self) -> usize {
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

struct RequestInput {
    provider: VertexCapabilityProvider,
    model: String,
    messages: Vec<Message>,
    optional_params: Object,
    litellm_params: Object,
    modify_params: bool,
    supports: Supports,
}

struct Harness {
    runtime: Runtime,
    counter: Counter,
    catalog: Arc<ModelCatalog>,
    hooks: Hooks,
    fetcher: MapFetch,
    transport: NoTransport,
    now: Timestamp,
}

impl Harness {
    fn new() -> Self {
        Self {
            runtime: Builder::new_current_thread()
                .build()
                .expect("a fixture runtime"),
            counter: counter(),
            catalog: BundledCatalog::default().get(),
            hooks: hooks(),
            fetcher: MapFetch::default(),
            transport: NoTransport::default(),
            now: Timestamp::from_second(NOW_SECOND).expect("fixture clock"),
        }
    }

    fn with_fetch(mut self, urls: &[(&str, &str)]) -> Self {
        self.fetcher.with_urls(urls);
        self
    }

    fn run(&self, input: &RequestInput) -> Result<Transformed, RequestError> {
        self.call(input, false)
    }

    fn run_async(&self, input: &RequestInput) -> Result<Transformed, RequestError> {
        self.call(input, true)
    }

    fn call(&self, input: &RequestInput, asynchronous: bool) -> Result<Transformed, RequestError> {
        let auth_token = SecretString::from("token-abc");
        let extra_headers = BTreeMap::new();
        let lookup = FixtureLookup {
            supports: input.supports,
        };
        let request = CallRequest {
            cache: CacheCreateRequest {
                provider: cache_provider(input.provider),
                model: &input.model,
                messages: &input.messages,
                api_base: None,
                project: Some("project"),
                location: Some("us-central1"),
                auth_token: Some(&auth_token),
                gemini_api_key: None,
                extra_headers: &extra_headers,
                deadline: Duration::from_secs(5),
                cached_content: None,
                disable_transform: false,
                now: self.now,
                minimum_token_override: None,
                counter: &self.counter,
                catalog: self.catalog.as_ref(),
                capabilities: &lookup,
                fetcher: &self.fetcher,
            },
            optional_params: &input.optional_params,
            litellm_params: &input.litellm_params,
            modify_params: input.modify_params,
            schema_template: None,
            capabilities: &lookup,
        };
        self.runtime.block_on(async {
            if asynchronous {
                transform_request_body_async(&self.transport, request, &self.hooks).await
            } else {
                transform_request_body(&self.transport, request, &self.hooks).await
            }
        })
    }
}

const fn cache_provider(provider: VertexCapabilityProvider) -> CacheProvider {
    match provider {
        VertexCapabilityProvider::VertexAi => CacheProvider::Vertex,
        VertexCapabilityProvider::VertexAiBeta => CacheProvider::VertexBeta,
        VertexCapabilityProvider::Gemini => CacheProvider::Gemini,
    }
}

fn parse_messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).expect("fixture messages")
}

fn parse_object(source: &str) -> Object {
    serde_json::from_str(source).expect("fixture object")
}

fn parse_json(source: &str) -> Json {
    serde_json::from_str(source).expect("fixture json")
}

fn vertex_input(model: &str, messages: &str, optional_params: &str) -> RequestInput {
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

fn assert_body(transformed: &Transformed, expected: &str) {
    assert_eq!(Json::Object(transformed.body.clone()), parse_json(expected));
}

fn run_vertex(optional_params: &str) -> (Harness, Result<Transformed, RequestError>) {
    let harness = Harness::new();
    let input = vertex_input("gemini-2.5-pro", USER_MESSAGES, optional_params);
    let result = harness.run(&input);
    (harness, result)
}

fn assert_no_io(harness: &Harness) {
    assert_eq!(harness.fetcher.calls(), 0);
    assert_eq!(harness.transport.calls(), 0);
}

#[rstest]
#[case::maps_after_extra_body(
    r#"{"temperature":0.3,"extra_body":{"generationConfig":{"response_mime_type":"application/json","response_json_schema":{"type":"object"}},"tools":[{"googleMaps":{}}]}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"tools":[{"googleMaps":{}}],"generationConfig":{"temperature":0.3,"responseFormat":{"text":{"mimeType":"APPLICATION_JSON","schema":{"type":"object"}}}}}"#
)]
#[case::maps_uses_response_schema(
    r#"{"response_schema":{"type":"object"},"extra_body":{"generationConfig":{"response_mime_type":"application/json"},"tools":[{"googleMaps":{}}]}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"tools":[{"googleMaps":{}}],"generationConfig":{"responseFormat":{"text":{"mimeType":"APPLICATION_JSON","schema":{"type":"object"}}}}}"#
)]
#[case::maps_without_json_mime(
    r#"{"response_mime_type":"text/plain","extra_body":{"tools":[{"googleMaps":{}}]}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"tools":[{"googleMaps":{}}],"generationConfig":{"response_mime_type":"text/plain"}}"#
)]
#[case::maps_without_google_maps(
    r#"{"extra_body":{"generationConfig":{"response_mime_type":"application/json","response_json_schema":{"type":"object"}}}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"response_mime_type":"application/json","response_json_schema":{"type":"object"}}}"#
)]
#[case::native_schema_stays(
    r#"{"response_schema":{"type":"object"},"temperature":0.1}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"response_schema":{"type":"object"},"temperature":0.1}}"#
)]
#[case::service_tier_default(
    r#"{"service_tier":"DEFAULT"}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"serviceTier":"standard"}"#
)]
#[case::service_tier_lowers(
    r#"{"service_tier":"Flex"}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"serviceTier":"flex"}"#
)]
#[case::service_tier_empty(
    r#"{"service_tier":""}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"serviceTier":""}"#
)]
#[case::service_tier_null(
    r#"{"service_tier":null}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#
)]
#[case::output_config_removed(
    r#"{"output_config":{"thinking_config":{"thinking_budget":5}},"temperature":0.1}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"temperature":0.1}}"#
)]
#[case::tools_null(
    r#"{"tools":null}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#
)]
#[case::empty_tools_are_sent(
    r#"{"tools":[]}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"tools":[]}"#
)]
#[case::tool_choice_null(
    r#"{"tool_choice":null}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#
)]
#[case::string_tool_choice_passes_through(
    r#"{"tool_choice":"auto"}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"toolConfig":"auto"}"#
)]
#[case::empty_tool_choice_is_sent(
    r#"{"tool_choice":{}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"toolConfig":{}}"#
)]
#[case::safety_settings_null(
    r#"{"safety_settings":null}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#
)]
#[case::generation_config_keeps_the_supplied_spelling(
    r#"{"top_p":0.9,"topP":0.8,"unknown_param":"x","max_tokens":16,"responseFormat":{"text":{"mimeType":"TEXT"}}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"top_p":0.9,"topP":0.8,"responseFormat":{"text":{"mimeType":"TEXT"}}}}"#
)]
#[case::internal_extra_body_keys(
    r#"{"temperature":0.1,"safety_settings":[{"category":"A"}],"extra_body":{"cache":{"probe":"a"},"tags":["x"],"generationConfig":{"topK":5},"safetySettings":[{"category":"B"}]}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"safetySettings":[{"category":"B"}],"generationConfig":{"temperature":0.1,"topK":5}}"#
)]
#[case::extra_body_replaces_scalar(
    r#"{"service_tier":"default","extra_body":{"serviceTier":"flex"}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"serviceTier":"flex"}"#
)]
#[case::extra_body_null(
    r#"{"extra_body":null}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#
)]
#[case::include_server_side_with_choice(
    r#"{"tool_choice":{"functionCallingConfig":{"mode":"AUTO"}},"include_server_side_tool_invocations":true}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"toolConfig":{"functionCallingConfig":{"mode":"AUTO"},"includeServerSideToolInvocations":true}}"#
)]
#[case::include_server_side_alone(
    r#"{"include_server_side_tool_invocations":true}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"toolConfig":{"includeServerSideToolInvocations":true}}"#
)]
#[case::include_server_side_false(
    r#"{"tool_choice":{"functionCallingConfig":{"mode":"NONE"}},"include_server_side_tool_invocations":false}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"toolConfig":{"functionCallingConfig":{"mode":"NONE"}}}"#
)]
#[case::null_config_value_takes_the_default(
    r#"{"temperature":null}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#
)]
#[case::json_schema_wins_over_schema(
    r#"{"extra_body":{"generationConfig":{"response_mime_type":"application/json","response_json_schema":{"type":"object","title":"json"},"response_schema":{"type":"object","title":"schema"}},"tools":[{"googleMaps":{}}]}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"tools":[{"googleMaps":{}}],"generationConfig":{"response_schema":{"type":"object","title":"schema"},"responseFormat":{"text":{"mimeType":"APPLICATION_JSON","schema":{"type":"object","title":"json"}}}}}"#
)]
#[case::maps_retrieval_merges_into_tool_config(
    r#"{"toolConfig":{"retrievalConfig":{"latLng":{"latitude":48.8,"longitude":2.3}},"functionCallingConfig":{"mode":"NONE"}},"tool_choice":{"functionCallingConfig":{"mode":"AUTO"}}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"toolConfig":{"retrievalConfig":{"latLng":{"latitude":48.8,"longitude":2.3}},"functionCallingConfig":{"mode":"AUTO"}}}"#
)]
#[case::mapped_tool_config_alone(
    r#"{"toolConfig":{"retrievalConfig":{"latLng":{"latitude":48.8,"longitude":2.3}}}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"toolConfig":{"retrievalConfig":{"latLng":{"latitude":48.8,"longitude":2.3}}}}"#
)]
fn body_cases(#[case] optional_params: &str, #[case] expected: &str) {
    let (harness, result) = run_vertex(optional_params);
    assert_body(&result.expect("the body builds"), expected);
    assert_no_io(&harness);
}

#[rstest]
#[case::legacy_schema_prompt(
    r#"{"response_schema":{"type":"object"},"temperature":0.1}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"},{"text":"Use this JSON schema: \n    ```json \n    {'type': 'object'}\n    ```"}]}],"generationConfig":{"temperature":0.1}}"#
)]
#[case::null_schema_takes_the_default(
    r#"{"response_schema":null,"temperature":0.1}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"temperature":0.1}}"#
)]
fn body_cases_without_native_schema(#[case] optional_params: &str, #[case] expected: &str) {
    let harness = Harness::new();
    let mut input = vertex_input("gemma-2.5-pro", USER_MESSAGES, optional_params);
    input.supports.response_schema = false;
    let transformed = harness.run(&input).expect("the body builds");
    assert_body(&transformed, expected);
}

#[rstest]
#[case::cache_modify_false(
    false,
    r#"{"cached_content":"cached-1","tools":[{"functionDeclarations":[{"name":"f"}]}],"tool_choice":{"functionCallingConfig":{"mode":"AUTO"}},"temperature":0.2}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"system_instruction":{"parts":[{"text":"rules"}]},"tools":[{"functionDeclarations":[{"name":"f"}]}],"toolConfig":{"functionCallingConfig":{"mode":"AUTO"}},"generationConfig":{"temperature":0.2},"cachedContent":"cached-1"}"#
)]
#[case::cache_modify_true(
    true,
    r#"{"cached_content":"cached-1","tools":[{"functionDeclarations":[{"name":"f"}]}],"tool_choice":{"functionCallingConfig":{"mode":"AUTO"}},"temperature":0.2}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"temperature":0.2},"cachedContent":"cached-1"}"#
)]
#[case::cached_keeps_safety_settings(
    true,
    r#"{"cached_content":"cached-1","tools":[{"googleSearch":{}}],"safety_settings":[{"category":"HARM_CATEGORY_HARASSMENT","threshold":"BLOCK_NONE"}]}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"safetySettings":[{"category":"HARM_CATEGORY_HARASSMENT","threshold":"BLOCK_NONE"}],"cachedContent":"cached-1"}"#
)]
#[case::cached_absent_with_modify_params(
    true,
    r#"{"tools":[{"googleSearch":{}}]}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"system_instruction":{"parts":[{"text":"rules"}]},"tools":[{"googleSearch":{}}]}"#
)]
#[case::include_server_side_dropped_with_cache(
    true,
    r#"{"cached_content":"cached-1","include_server_side_tool_invocations":true}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"cachedContent":"cached-1"}"#
)]
#[case::cached_content_empty_is_sent(
    false,
    r#"{"cached_content":""}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"system_instruction":{"parts":[{"text":"rules"}]},"cachedContent":""}"#
)]
#[case::cached_content_null_is_absent(
    false,
    r#"{"cached_content":null}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"system_instruction":{"parts":[{"text":"rules"}]}}"#
)]
#[case::mapped_tool_config_dropped_with_cache(
    true,
    r#"{"cached_content":"cached-1","toolConfig":{"retrievalConfig":{"latLng":{"latitude":48.8,"longitude":2.3}}}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"cachedContent":"cached-1"}"#
)]
fn body_cases_with_cache(
    #[case] modify_params: bool,
    #[case] optional_params: &str,
    #[case] expected: &str,
) {
    let harness = Harness::new();
    let mut input = vertex_input("gemini-2.5-pro", SYSTEM_USER_MESSAGES, optional_params);
    input.modify_params = modify_params;
    let transformed = harness.run(&input).expect("the body builds");
    assert_body(&transformed, expected);
    assert_eq!(harness.transport.calls(), 0);
}

#[rstest]
#[case::studio_omits_labels(
    VertexCapabilityProvider::Gemini,
    r#"{"labels":{"team":"a"}}"#,
    "{}",
    USER_CONTENTS,
    "{}"
)]
#[case::vertex_labels(
    VertexCapabilityProvider::VertexAi,
    r#"{"labels":{"team":"a"}}"#,
    "{}",
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"labels":{"team":"a"}}"#,
    "{}"
)]
#[case::labels_from_metadata(
    VertexCapabilityProvider::VertexAi,
    "{}",
    r#"{"metadata":{"requester_metadata":{"owner":"b"}}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"labels":{"owner":"b"}}"#,
    r#"{"metadata":{"requester_metadata":{"owner":"b"}}}"#
)]
#[case::litellm_param_moves_to_metadata(
    VertexCapabilityProvider::VertexAi,
    r#"{"litellm_param_vertex_project":"p","temperature":0.1}"#,
    r#"{"metadata":{"user_api_key":"k"}}"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"temperature":0.1}}"#,
    r#"{"metadata":{"user_api_key":"k"},"litellm_param_vertex_project":"p"}"#
)]
fn body_cases_with_labels(
    #[case] provider: VertexCapabilityProvider,
    #[case] optional_params: &str,
    #[case] litellm_params: &str,
    #[case] expected: &str,
    #[case] expected_litellm_params: &str,
) {
    let harness = Harness::new();
    let mut input = vertex_input("gemini-2.5-pro", USER_MESSAGES, optional_params);
    input.provider = provider;
    input.litellm_params = parse_object(litellm_params);
    let transformed = harness.run(&input).expect("the body builds");
    assert_body(&transformed, expected);
    assert_eq!(
        Json::Object(transformed.litellm_params),
        parse_json(expected_litellm_params)
    );
}

#[rstest]
#[case::global_media_resolution(
    "gemini-2.5-pro",
    IMAGE_MESSAGES,
    r#"{"contents":[{"role":"user","parts":[{"text":"look"},{"inline_data":{"data":"YWJj","mime_type":"image/png"}}]}],"generationConfig":{"mediaResolution":"MEDIA_RESOLUTION_HIGH"}}"#
)]
#[case::global_media_resolution_skips_gemini_one(
    "gemini-1.5-pro",
    IMAGE_MESSAGES,
    r#"{"contents":[{"role":"user","parts":[{"text":"look"},{"inline_data":{"data":"YWJj","mime_type":"image/png"}}]}]}"#
)]
#[case::global_media_resolution_takes_the_highest(
    "gemini-2.5-pro",
    r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"data:image/png;base64,YWJj","detail":"low"}},{"type":"image_url","image_url":{"url":"data:image/png;base64,ZGVm","detail":"high"}}]}]"#,
    r#"{"contents":[{"role":"user","parts":[{"text":"look"},{"inline_data":{"data":"YWJj","mime_type":"image/png"}},{"inline_data":{"data":"ZGVm","mime_type":"image/png"}}]}],"generationConfig":{"mediaResolution":"MEDIA_RESOLUTION_HIGH"}}"#
)]
fn body_cases_with_media(#[case] model: &str, #[case] messages: &str, #[case] expected: &str) {
    let harness = Harness::new();
    let input = vertex_input(model, messages, "{}");
    let transformed = harness.run(&input).expect("the body builds");
    assert_body(&transformed, expected);
    assert_eq!(harness.fetcher.calls(), 0);
}

#[rstest]
#[case::system_extracted(
    true,
    r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"system_instruction":{"parts":[{"text":"rules"}]}}"#
)]
#[case::system_merged_when_unsupported(
    false,
    r#"{"contents":[{"role":"user","parts":[{"text":"rules"},{"text":"hi"}]}]}"#
)]
fn body_cases_with_system_messages(#[case] system_message: bool, #[case] expected: &str) {
    let harness = Harness::new();
    let mut input = vertex_input("gemini-2.5-pro", SYSTEM_USER_MESSAGES, "{}");
    input.supports.system_message = system_message;
    let transformed = harness.run(&input).expect("the body builds");
    assert_body(&transformed, expected);
}

#[rstest]
#[case::tools_must_be_a_list(r#"{"tools":"functions"}"#, "tools")]
#[case::service_tier_must_be_a_string(r#"{"service_tier":5}"#, "service_tier")]
#[case::include_server_side_must_be_a_boolean(
    r#"{"include_server_side_tool_invocations":"yes"}"#,
    "include_server_side_tool_invocations"
)]
#[case::extra_body_must_be_an_object(r#"{"extra_body":"x"}"#, "extra_body")]
#[case::cached_content_must_be_a_string(r#"{"cached_content":5}"#, "cached_content")]
#[case::tool_choice_must_be_an_object(
    r#"{"tool_choice":"auto","include_server_side_tool_invocations":true}"#,
    "tool_choice"
)]
#[case::tool_config_must_be_an_object(r#"{"toolConfig":"auto"}"#, "toolConfig")]
fn body_cases_errors(#[case] optional_params: &str, #[case] field: &str) {
    let (harness, result) = run_vertex(optional_params);
    let error = result.expect_err("the parameters are refused");
    assert!(error.to_string().contains(field), "{error}");
    assert_no_io(&harness);
}

fn media_input(provider: VertexCapabilityProvider, messages: &str) -> RequestInput {
    let mut input = vertex_input("gemini-2.5-pro", messages, "{}");
    input.provider = provider;
    input
}

#[test]
fn async_keeps_the_cached_content() {
    let harness = Harness::new();
    let input = vertex_input(
        "gemini-2.5-pro",
        USER_MESSAGES,
        r#"{"cached_content":"cached-1"}"#,
    );
    let transformed = harness.run_async(&input).expect("the body builds");
    assert_body(
        &transformed,
        r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"cachedContent":"cached-1"}"#,
    );
    assert_eq!(harness.transport.calls(), 0);
}

fn run_entry(
    harness: &Harness,
    input: &RequestInput,
    asynchronous: bool,
) -> Result<Transformed, RequestError> {
    if asynchronous {
        harness.run_async(input)
    } else {
        harness.run(input)
    }
}

fn assistant_image_messages(image_url: &str) -> String {
    format!(
        r#"[{{"role":"user","content":"q"}},{{"role":"assistant","content":"a","images":[{{"type":"image_url","image_url":{image_url},"index":0}}]}},{{"role":"user","content":"next"}}]"#
    )
}

fn assert_fetched(harness: &Harness, asynchronous: bool, urls: &[&str]) {
    let expected: Vec<String> = urls.iter().map(|url| (*url).to_owned()).collect();
    assert_eq!(harness.fetcher.calls(), expected.len());
    if asynchronous {
        assert_eq!(harness.fetcher.batches(), vec![expected]);
        assert!(harness.fetcher.converted().is_empty());
    } else {
        assert_eq!(harness.fetcher.converted(), expected);
        assert!(harness.fetcher.batches().is_empty());
    }
}

#[rstest]
#[case::sequential(false, REMOTE_HTTPS)]
#[case::concurrent(true, REMOTE_HTTPS)]
#[case::http_extension(false, REMOTE_HTTP_PNG)]
#[case::http_extension_concurrent(true, REMOTE_HTTP_PNG)]
fn media_download(#[case] asynchronous: bool, #[case] url: &str) {
    let harness = Harness::new().with_fetch(&[(url, FETCHED)]);
    let messages = format!(
        r#"[{{"role":"user","content":[{{"type":"text","text":"what"}},{{"type":"image_url","image_url":{{"url":"{url}"}}}}]}}]"#
    );
    let input = media_input(VertexCapabilityProvider::VertexAi, &messages);
    let transformed = run_entry(&harness, &input, asynchronous).expect("the media is fetched");
    assert_body(&transformed, INLINE_CAT_BODY);
    assert_fetched(&harness, asynchronous, &[url]);
    assert_eq!(harness.transport.calls(), 0);
}

#[rstest]
#[case::format(VertexCapabilityProvider::VertexAi, "format", "image/png")]
#[case::format_beta(VertexCapabilityProvider::VertexAiBeta, "format", "image/png")]
#[case::mime_type(VertexCapabilityProvider::VertexAi, "mime_type", "image/jpeg")]
#[case::content_type(VertexCapabilityProvider::VertexAi, "content_type", "image/webp")]
fn media_https_hint_passes_mime(
    #[case] provider: VertexCapabilityProvider,
    #[case] key: &str,
    #[case] mime: &str,
) {
    let harness = Harness::new();
    let messages = format!(
        r#"[{{"role":"user","content":[{{"type":"text","text":"what"}},{{"type":"image_url","image_url":{{"url":"https://example.com/cat","{key}":"{mime}"}}}}]}}]"#
    );
    let input = media_input(provider, &messages);
    let transformed = harness.run(&input).expect("the hinted reference stays");
    let expected = format!(
        r#"{{"contents":[{{"role":"user","parts":[{{"text":"what"}},{{"file_data":{{"mime_type":"{mime}","file_uri":"https://example.com/cat"}}}}]}}]}}"#
    );
    assert_body(&transformed, &expected);
    assert_no_io(&harness);
}

#[test]
fn media_studio_https_extension() {
    let harness = Harness::new().with_fetch(&[(REMOTE_HTTPS_PNG, FETCHED)]);
    let input = media_input(
        VertexCapabilityProvider::Gemini,
        r#"[{"role":"user","content":[{"type":"text","text":"what"},{"type":"image_url","image_url":{"url":"https://example.com/cat.png"}}]}]"#,
    );
    let transformed = harness.run(&input).expect("the media is downloaded");
    assert_body(&transformed, INLINE_CAT_BODY);
    assert_fetched(&harness, false, &[REMOTE_HTTPS_PNG]);
}

#[rstest]
#[case::vertex(VertexCapabilityProvider::VertexAi)]
#[case::studio(VertexCapabilityProvider::Gemini)]
fn media_files_api_uri_passthrough(#[case] provider: VertexCapabilityProvider) {
    let harness = Harness::new();
    let input = media_input(
        provider,
        r#"[{"role":"user","content":[{"type":"text","text":"read"},{"type":"image_url","image_url":{"url":"https://generativelanguage.googleapis.com/v1beta/files/abc123"}}]}]"#,
    );
    let transformed = harness
        .run(&input)
        .expect("the uploaded uri stays a reference");
    assert_body(
        &transformed,
        r#"{"contents":[{"role":"user","parts":[{"text":"read"},{"file_data":{"file_uri":"https://generativelanguage.googleapis.com/v1beta/files/abc123"}}]}]}"#,
    );
    assert_no_io(&harness);
}

#[test]
fn media_data_url_not_fetched() {
    let harness = Harness::new();
    let input = media_input(
        VertexCapabilityProvider::VertexAi,
        r#"[{"role":"user","content":[{"type":"text","text":"what"},{"type":"image_url","image_url":{"url":"data:image/png;base64,YWJj"}}]}]"#,
    );
    let transformed = harness.run(&input).expect("the inline data stays");
    assert_body(&transformed, INLINE_CAT_BODY);
    assert_no_io(&harness);
}

#[rstest]
#[case::vertex_sync(VertexCapabilityProvider::VertexAi, false)]
#[case::vertex_async(VertexCapabilityProvider::VertexAi, true)]
#[case::studio_sync(VertexCapabilityProvider::Gemini, false)]
#[case::studio_async(VertexCapabilityProvider::Gemini, true)]
fn media_assistant_image_fetches(
    #[case] provider: VertexCapabilityProvider,
    #[case] asynchronous: bool,
) {
    let harness = Harness::new().with_fetch(&[(REMOTE_HTTP, FETCHED)]);
    let messages =
        assistant_image_messages(r#"{"url":"http://example.com/cat","format":"image/jpeg"}"#);
    let input = media_input(provider, &messages);
    let transformed = run_entry(&harness, &input, asynchronous).expect("the image is downloaded");
    assert_body(&transformed, ASSISTANT_JPEG_BODY);
    assert_fetched(&harness, asynchronous, &[REMOTE_HTTP]);
}

#[test]
fn media_assistant_image_hint_passthrough() {
    let harness = Harness::new();
    let messages =
        assistant_image_messages(r#"{"url":"https://example.com/a","mime_type":"image/webp"}"#);
    let input = media_input(VertexCapabilityProvider::VertexAi, &messages);
    let transformed = harness
        .run(&input)
        .expect("the hinted image stays a reference");
    assert_body(
        &transformed,
        r#"{"contents":[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"},{"file_data":{"mime_type":"image/webp","file_uri":"https://example.com/a"}}]},{"role":"user","parts":[{"text":"next"}]}]}"#,
    );
    assert_no_io(&harness);
}

#[test]
fn media_assistant_image_string_skipped() {
    let harness = Harness::new();
    let messages = assistant_image_messages(r#""https://example.com/a""#);
    let input = media_input(VertexCapabilityProvider::VertexAi, &messages);
    let transformed = harness.run(&input).expect("the string is skipped");
    assert_body(&transformed, ASSISTANT_TEXT_BODY);
    assert_no_io(&harness);
}

#[test]
fn media_assistant_image_hint_survives_a_shared_download() {
    let harness = Harness::new().with_fetch(&[(REMOTE_A, FETCHED)]);
    let messages = r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","images":[{"type":"image_url","image_url":{"url":"https://example.com/a"}},{"type":"image_url","image_url":{"url":"https://example.com/a","format":"image/jpeg"}},{"type":"image_url","image_url":{"url":"https://example.com/a"}}]},{"role":"user","content":"next"}]"#;
    let input = media_input(VertexCapabilityProvider::VertexAi, messages);
    let transformed = harness.run(&input).expect("the media is downloaded");
    assert_body(
        &transformed,
        r#"{"contents":[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"},{"inline_data":{"data":"YWJj","mime_type":"image/png"}},{"file_data":{"mime_type":"image/jpeg","file_uri":"https://example.com/a"}},{"inline_data":{"data":"YWJj","mime_type":"image/png"}}]},{"role":"user","parts":[{"text":"next"}]}]}"#,
    );
    assert_fetched(&harness, false, &[REMOTE_A]);
}

#[test]
fn media_studio_image_extension() {
    let harness = Harness::new();
    let messages = assistant_image_messages(r#"{"url":"https://example.com/a.png"}"#);
    let input = media_input(VertexCapabilityProvider::Gemini, &messages);
    let transformed = harness.run(&input).expect("the image stays a reference");
    assert_body(
        &transformed,
        r#"{"contents":[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"},{"file_data":{"mime_type":"image/png","file_uri":"https://example.com/a.png"}}]},{"role":"user","parts":[{"text":"next"}]}]}"#,
    );
    assert_no_io(&harness);
}

#[rstest]
#[case::sequential(false)]
#[case::concurrent(true)]
fn media_refused_fetch_fails_once(#[case] asynchronous: bool) {
    let harness = Harness::new();
    let input = media_input(
        VertexCapabilityProvider::VertexAi,
        r#"[{"role":"user","content":[{"type":"text","text":"what"},{"type":"image_url","image_url":{"url":"http://example.com/cat"}}]}]"#,
    );
    let error = run_entry(&harness, &input, asynchronous).expect_err("the download is refused");
    let url = match error {
        RequestError::Inline(MediaError::Fetch(ImageFetchError::Attempts { url })) => url,
        other @ (RequestError::Cache(_) | RequestError::Body(_) | RequestError::Inline(_)) => {
            panic!("a refused download is an inline media error, got {other:?}")
        }
    };
    assert_eq!(url, REMOTE_HTTP);
    assert_fetched(&harness, asynchronous, &[REMOTE_HTTP]);
    assert_eq!(harness.transport.calls(), 0);
}

#[test]
fn media_studio_file_fetch_failure() {
    let harness = Harness::new();
    let input = media_input(VertexCapabilityProvider::Gemini, STUDIO_FILE_MESSAGES);
    let error = harness.run(&input).expect_err("the download is refused");
    let url = match error {
        RequestError::Inline(MediaError::Fetch(ImageFetchError::Attempts { url })) => url,
        other @ (RequestError::Cache(_) | RequestError::Body(_) | RequestError::Inline(_)) => {
            panic!("a refused studio file download is not inline media: {other:?}")
        }
    };
    assert_eq!(url, REMOTE_PDF);
    assert_fetched(&harness, false, &[REMOTE_PDF]);
    assert_eq!(harness.transport.calls(), 0);
}

#[test]
fn media_sequential_fetch_order() {
    let harness = Harness::new().with_fetch(&[
        (REMOTE_A, FETCHED),
        (REMOTE_B, "data:image/png;base64,ZGVm"),
    ]);
    let input = media_input(
        VertexCapabilityProvider::VertexAi,
        r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"https://example.com/a"}},{"type":"image_url","image_url":{"url":"https://example.com/b"}}]}]"#,
    );
    let transformed = harness.run(&input).expect("both media are downloaded");
    assert_body(
        &transformed,
        r#"{"contents":[{"role":"user","parts":[{"text":"look"},{"inline_data":{"data":"YWJj","mime_type":"image/png"}},{"inline_data":{"data":"ZGVm","mime_type":"image/png"}}]}]}"#,
    );
    assert_fetched(&harness, false, &[REMOTE_A, REMOTE_B]);
}

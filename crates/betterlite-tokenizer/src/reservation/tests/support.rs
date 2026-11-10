use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use tokio::time::Instant;

use crate::count::Counter;
use crate::count::request::{TextInput, TokenRequest};
use crate::count::settings::Settings;
use crate::engine::Engine;
use crate::engine::error::EngineError;
use crate::engine::fetch::TiktokenFetcher;
use crate::engine::hub::HubDownloader;
use crate::engine::limits::{BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService};
use crate::engine::selection::Selector;
use crate::reservation::RequestBody;

const CATALOG: &[u8] = br#"{
  "gpt-3.5-turbo": {"litellm_provider": "openai"},
  "gpt-3.5-turbo-0301": {"litellm_provider": "openai"},
  "text-davinci-001": {"litellm_provider": "openai"}
}"#;

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

pub(crate) fn body(source: &str) -> RequestBody {
    serde_json::from_str(source).unwrap()
}

pub(crate) async fn text_tokens(counter: &Counter, model: &str, text: &str) -> usize {
    let input = TextInput::One(text.to_owned());
    let request = TokenRequest {
        model,
        custom_tokenizer: None,
        text: Some(&input),
        messages: None,
        count_response_tokens: None,
        tools: None,
        tool_choice: None,
    };
    counter.count(&request).await.unwrap()
}

pub(crate) fn bundled_counter(directory: &Path) -> Counter {
    let service = service(4, Arc::new(InlineCounter));
    let fetcher = bundled_fetcher(Arc::clone(&service));
    assemble(directory, service, fetcher, BundledCatalog::default().get())
}

pub(crate) fn legacy_counter(directory: &Path) -> Counter {
    let service = service(4, Arc::new(InlineCounter));
    let fetcher = bundled_fetcher(Arc::clone(&service));
    assemble(directory, service, fetcher, catalog())
}

pub(crate) fn dead_remote_counter(directory: &Path) -> Counter {
    let service = service(4, Arc::new(InlineCounter));
    let fetcher = dead_remote_fetcher(Arc::clone(&service));
    assemble(directory, service, fetcher, catalog())
}

pub(crate) fn counter_with_facility(
    directory: &Path,
    facility: Arc<dyn BlockingCounter>,
    max_concurrent: usize,
) -> Counter {
    let service = service(max_concurrent, facility);
    let fetcher = bundled_fetcher(Arc::clone(&service));
    assemble(directory, service, fetcher, catalog())
}

fn catalog() -> Arc<ModelCatalog> {
    Arc::new(ModelCatalog::parse(CATALOG, UnknownFields::Reject).unwrap())
}

fn bundled_fetcher(service: Arc<CountingService>) -> Arc<TiktokenFetcher> {
    Arc::new(TiktokenFetcher::new(client(), None, service))
}

fn dead_remote_fetcher(service: Arc<CountingService>) -> Arc<TiktokenFetcher> {
    Arc::new(TiktokenFetcher::new(client(), None, service).with_base(
        "http://127.0.0.1:1".to_owned(),
        [String::new(), String::new(), String::new()],
    ))
}

fn assemble(
    directory: &Path,
    service: Arc<CountingService>,
    fetcher: Arc<TiktokenFetcher>,
    catalog: Arc<ModelCatalog>,
) -> Counter {
    let hub = hub_settings(directory);
    let downloader = Arc::new(HubDownloader::new(
        Arc::new(hub.clone()),
        resolver(&hub),
        Arc::new(Instant::now),
    ));
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        downloader,
        Arc::clone(&catalog),
        Arc::clone(&service),
        false,
    ));
    Counter::new(selector, fetcher, service, catalog, Settings::default())
}

fn service(max_concurrent: usize, facility: Arc<dyn BlockingCounter>) -> Arc<CountingService> {
    let limits = Settings::default().encode_limits().unwrap();
    Arc::new(CountingService::new(max_concurrent, facility, limits))
}

fn client() -> Arc<ProxiedClient> {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    Arc::new(ProxiedClient::outbound(&policy).unwrap())
}

fn hub_settings(directory: &Path) -> HubSettings {
    HubSettings {
        endpoint: "http://127.0.0.1:1".to_owned(),
        offline: OfflineSetting::Online,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: directory.to_path_buf(),
        hub_cache: directory.to_path_buf(),
        token_path: directory.join("token"),
        stored_tokens_path: directory.join("stored_tokens"),
        transfer: TransferSetting::Standard,
    }
}

fn resolver(settings: &HubSettings) -> Arc<HubTokenResolver> {
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    Arc::new(HubTokenResolver::new(
        settings.clone(),
        OidcSettings::default(),
        client(),
        reader,
        clock,
    ))
}

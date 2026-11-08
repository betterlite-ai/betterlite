use std::collections::BTreeMap;
use std::num::NonZeroUsize;
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
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};

use tokio::time::Instant;

use crate::count::Counter;
use crate::count::request::TextInput;
use crate::count::settings::Settings;
use crate::count::wire::{CustomTokenizerWire, TokenRequestWire};
use crate::engine::Engine;
use crate::engine::error::EngineError;
use crate::engine::fetch::TiktokenFetcher;
use crate::engine::hub::{Clock, HubDownloader};
use crate::engine::limits::{BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService};
use crate::engine::selection::Selector;

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

pub(crate) fn client() -> Arc<ProxiedClient> {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    Arc::new(ProxiedClient::outbound(&policy).unwrap())
}

pub(crate) fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

pub(crate) fn service(settings: Settings) -> Arc<CountingService> {
    Arc::new(CountingService::new(
        4,
        Arc::new(InlineCounter),
        settings.encode_limits().unwrap(),
    ))
}

pub(crate) fn hub_settings(endpoint: &str, directory: &Path) -> HubSettings {
    HubSettings {
        endpoint: endpoint.to_owned(),
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
    let oidc = OidcSettings {
        resource: None,
        id_token: None,
        github_actions: false,
        request_url: None,
        request_token: None,
        device_code_client_id: String::new(),
    };
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    Arc::new(HubTokenResolver::new(
        settings.clone(),
        oidc,
        client(),
        reader,
        clock,
    ))
}

fn system_clock() -> Arc<Clock> {
    Arc::new(Instant::now)
}

pub(crate) fn settings(max_exact: usize, chunk: usize, disabled: bool) -> Settings {
    Settings {
        disabled,
        max_exact_chars: NonZeroUsize::new(max_exact).unwrap(),
        chunk_chars: NonZeroUsize::new(chunk).unwrap(),
    }
}

pub(crate) async fn count_request(counter: &Counter, request: &str) -> Result<usize, String> {
    let wire = TokenRequestWire::try_from(request.as_bytes()).map_err(|error| error.to_string())?;
    let selected = wire.selected().map_err(|error| error.to_string())?;
    counter
        .count(&wire.request(selected.as_ref()))
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn count_with_tokenizer(
    counter: &Counter,
    model: &str,
    text: &str,
    tokenizer: CustomTokenizerWire,
) -> Result<usize, String> {
    let wire = TokenRequestWire {
        model: model.to_owned(),
        custom_tokenizer: Some(tokenizer),
        text: Some(TextInput::One(text.to_owned())),
        messages: None,
        count_response_tokens: false,
        tools: None,
        tool_choice: None,
    };
    let selected = wire.selected().map_err(|error| error.to_string())?;
    counter
        .count(&wire.request(selected.as_ref()))
        .await
        .map_err(|error| error.to_string())
}

fn parts(
    settings: Settings,
) -> (
    Arc<TiktokenFetcher>,
    Arc<CountingService>,
    Arc<ModelCatalog>,
) {
    let service = service(settings);
    let fetcher = Arc::new(TiktokenFetcher::new(client(), None, Arc::clone(&service)));
    (fetcher, service, catalog())
}

pub(crate) fn selector(hub: &HubSettings, settings: Settings) -> Selector {
    let (fetcher, service, catalog) = parts(settings);
    let downloader = Arc::new(HubDownloader::new(
        Arc::new(hub.clone()),
        resolver(hub),
        system_clock(),
    ));
    Selector::new(fetcher, downloader, catalog, service, false)
}

pub(crate) fn counter(settings: Settings, directory: &Path) -> Counter {
    let hub = hub_settings("http://127.0.0.1:1", directory);
    let downloader = Arc::new(HubDownloader::new(
        Arc::new(hub.clone()),
        resolver(&hub),
        system_clock(),
    ));
    let (fetcher, service, catalog) = parts(settings);
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        downloader,
        Arc::clone(&catalog),
        Arc::clone(&service),
        false,
    ));
    Counter::new(selector, fetcher, service, catalog, settings)
}

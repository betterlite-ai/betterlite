use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::SystemHubClock;
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::settings::Settings;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService,
};
use betterlite_tokenizer::engine::selection::Selector;
use tokio::time::Instant;

use super::errors;

pub(super) struct Session {
    pub(super) counter: Counter,
    pub(super) catalog: Arc<ModelCatalog>,
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

impl Session {
    pub(super) fn build() -> Result<Self, Box<crate::helpers::HelperError>> {
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| errors::value_error(error.to_string()))?;
        let client = Arc::new(
            ProxiedClient::outbound(&Arc::new(policy))
                .map_err(|error| errors::value_error(error.to_string()))?,
        );
        let settings = Settings::default();
        let service = Arc::new(CountingService::new(
            4,
            Arc::new(InlineCounter),
            settings
                .encode_limits()
                .map_err(|error| errors::value_error(error.to_string()))?,
        ));
        let fetcher = Arc::new(TiktokenFetcher::new(
            Arc::clone(&client),
            None,
            Arc::clone(&service),
        ));
        let catalog = BundledCatalog::default().get();
        let selector = Arc::new(selector(&fetcher, client, Arc::clone(&service), &catalog));
        let counter = Counter::new(selector, fetcher, service, Arc::clone(&catalog), settings);
        Ok(Self { counter, catalog })
    }
}

fn selector(
    fetcher: &Arc<TiktokenFetcher>,
    client: Arc<ProxiedClient>,
    service: Arc<CountingService>,
    catalog: &Arc<ModelCatalog>,
) -> Selector {
    let hub = hub_settings();
    let clock = Arc::new(SystemHubClock::default());
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let tokens = Arc::new(HubTokenResolver::new(
        hub.clone(),
        OidcSettings::default(),
        client,
        reader,
        clock,
    ));
    let downloader = Arc::new(HubDownloader::new(
        Arc::new(hub),
        tokens,
        Arc::new(Instant::now),
    ));
    Selector::new(
        Arc::clone(fetcher),
        downloader,
        Arc::clone(catalog),
        service,
        true,
    )
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

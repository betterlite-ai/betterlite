use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use tokio::time::Instant;

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
use betterlite_models::catalog::BundledCatalog;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::default_encoding::{DefaultEncoding, DefaultEngine};
use betterlite_tokenizer::count::settings::Settings;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService,
};
use betterlite_tokenizer::engine::selection::Selector;

use crate::helpers::HelperError;

pub(super) struct Fixture {
    pub(super) counter: Counter,
    pub(super) default: DefaultEncoding,
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

pub(super) fn fixture(settings: Settings, directory: &Path) -> Result<Fixture, Box<HelperError>> {
    let client = client()?;
    let limits = settings
        .encode_limits()
        .map_err(|error| failure(error.to_string()))?;
    let facility: Arc<dyn BlockingCounter> = Arc::new(InlineCounter);
    let service = Arc::new(CountingService::new(4, facility, limits));
    let fetcher = Arc::new(TiktokenFetcher::new(
        Arc::clone(&client),
        None,
        Arc::clone(&service),
    ));
    let catalog = BundledCatalog::default().get();
    let hub = hub_settings(directory);
    let downloader = Arc::new(HubDownloader::new(
        Arc::new(hub.clone()),
        resolver(&hub, client),
        Arc::new(Instant::now),
    ));
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        downloader,
        Arc::clone(&catalog),
        Arc::clone(&service),
        false,
    ));
    let concrete: Arc<Selector> = Arc::clone(&selector);
    let default_loader: Arc<dyn DefaultEngine> = concrete;
    Ok(Fixture {
        counter: Counter::new(selector, fetcher, service, catalog, settings),
        default: DefaultEncoding::new(default_loader),
    })
}

fn client() -> Result<Arc<ProxiedClient>, Box<HelperError>> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .map_err(|error| failure(error.to_string()))?;
    let client =
        ProxiedClient::outbound(&Arc::new(policy)).map_err(|error| failure(error.to_string()))?;
    Ok(Arc::new(client))
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

fn resolver(settings: &HubSettings, client: Arc<ProxiedClient>) -> Arc<HubTokenResolver> {
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
        client,
        reader,
        clock,
    ))
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

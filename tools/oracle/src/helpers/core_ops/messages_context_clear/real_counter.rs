use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::Settings as HubSettings;
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::settings::Settings;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BuildJob, ConvertJob, CountJob, CountingService, EncodeLimits,
};
use betterlite_tokenizer::engine::selection::Selector;
use tokio::time::Instant;

use crate::helpers::HelperError;

use super::failure;

struct InlineCounter;

#[async_trait::async_trait]
impl betterlite_tokenizer::engine::limits::BlockingCounter for InlineCounter {
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

pub(super) fn real_counter() -> Result<Arc<Counter>, Box<HelperError>> {
    let service = Arc::new(CountingService::new(
        4,
        Arc::new(InlineCounter),
        EncodeLimits::default(),
    ));
    let client = client()?;
    let catalog = BundledCatalog::default().get();
    let fetcher = Arc::new(TiktokenFetcher::new(
        Arc::clone(&client),
        None,
        Arc::clone(&service),
    ));
    let downloader = hub(client);
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        downloader,
        Arc::clone(&catalog),
        Arc::clone(&service),
        false,
    ));
    Ok(Arc::new(Counter::new(
        selector,
        fetcher,
        service,
        catalog,
        Settings::default(),
    )))
}

fn client() -> Result<Arc<ProxiedClient>, Box<HelperError>> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).map_err(failure)?;
    let client = ProxiedClient::outbound(&Arc::new(policy)).map_err(failure)?;
    Ok(Arc::new(client))
}

fn hub(client: Arc<ProxiedClient>) -> Arc<HubDownloader> {
    let settings = HubSettings::default();
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let resolver = Arc::new(HubTokenResolver::new(
        settings.clone(),
        OidcSettings::default(),
        client,
        reader,
        clock,
    ));
    Arc::new(HubDownloader::new(
        Arc::new(settings),
        resolver,
        Arc::new(Instant::now),
    ))
}

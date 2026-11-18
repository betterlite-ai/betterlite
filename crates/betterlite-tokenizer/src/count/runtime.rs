use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::types::Config;
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::ModelCatalog;
use tokio::time::Instant;

use crate::count::Counter;
use crate::count::error::CountError;
use crate::count::settings::Settings as CountSettings;
use crate::engine::Engine;
use crate::engine::error::EngineError;
use crate::engine::fetch::TiktokenFetcher;
use crate::engine::hub::HubDownloader;
use crate::engine::limits::{BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService};
use crate::engine::selection::Selector;

const TOKEN_COUNT_CONCURRENCY: usize = 4;

// the chain the tokenizer's own tests build: a bundled vocabulary needs no download, and the hub is reached only for a custom tokenizer
pub fn runtime_counter(
    config: &Config,
    client: &Arc<ProxiedClient>,
    catalog: &Arc<ModelCatalog>,
) -> Result<Counter, CountError> {
    let settings = CountSettings::from_config(&config.sections.token_counting);
    let limits = settings.encode_limits()?;
    let facility: Arc<dyn BlockingCounter> = Arc::new(InlineCounter);
    let service = Arc::new(CountingService::new(
        TOKEN_COUNT_CONCURRENCY,
        facility,
        limits,
    ));
    let fetcher = Arc::new(TiktokenFetcher::new(
        Arc::clone(client),
        None,
        Arc::clone(&service),
    ));
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        hub_downloader(config, client),
        Arc::clone(catalog),
        Arc::clone(&service),
        false,
    ));
    Ok(Counter::new(
        selector,
        service,
        Arc::clone(catalog),
        settings,
    ))
}

fn hub_downloader(config: &Config, client: &Arc<ProxiedClient>) -> Arc<HubDownloader> {
    let hub = config.sections.huggingface_hub.clone();
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let hub_clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let resolver = Arc::new(HubTokenResolver::new(
        hub.clone(),
        config.sections.huggingface_oidc.clone(),
        Arc::clone(client),
        reader,
        hub_clock,
    ));
    Arc::new(HubDownloader::new(
        Arc::new(hub),
        resolver,
        Arc::new(Instant::now),
    ))
}

// Vertex counts inside block_in_place; chat recovery counts on the request task like LiteLLM token_counter
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

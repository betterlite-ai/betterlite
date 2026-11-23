use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
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

use super::error::TokensError;

pub(crate) struct TokensRuntime {
    pub(crate) counter: Counter,
    pub(crate) selector: Arc<Selector>,
}

impl TokensRuntime {
    // the wheel has no config file, so the composition comes from the process environment on each call
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn build() -> Result<Self, TokensError> {
        let environment = process_environment();
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment)?;
        let config = ConfigLoader {
            lookup: environment,
        }
        .load("{}")?;
        let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy))?);
        Self::compose(&config, &client)
    }

    // the tokenizer_engines and token_counting sections carry LiteLLM's limit, cache directory and download flag
    #[cfg_attr(test, mutants::skip)]
    fn compose(config: &Config, client: &Arc<ProxiedClient>) -> Result<Self, TokensError> {
        let catalog: Arc<ModelCatalog> = BundledCatalog::default().get();
        let counting = CountSettings::from_config(&config.sections.token_counting);
        let engines = &config.sections.tokenizer_engines;
        let service = Arc::new(CountingService::new(
            engines.max_concurrent_counts,
            Arc::new(InlineCounter),
            counting.encode_limits()?,
        ));
        let fetcher = Arc::new(TiktokenFetcher::new(
            Arc::clone(client),
            engines.custom_tiktoken_cache_dir.clone(),
            Arc::clone(&service),
        ));
        let hub = Arc::new(HubDownloader::new(
            Arc::new(config.sections.huggingface_hub.clone()),
            hub_tokens(config, client),
            Arc::new(Instant::now),
        ));
        let selector = Arc::new(Selector::new(
            fetcher,
            hub,
            Arc::clone(&catalog),
            Arc::clone(&service),
            engines.disable_hf_tokenizer_download,
        ));
        let counter = Counter::new(Arc::clone(&selector), service, catalog, counting);
        Ok(Self { counter, selector })
    }
}

// the resolver's settings equal the ones an empty document loads, and no test has a stored or oidc token to resolve
#[cfg_attr(test, mutants::skip)]
fn hub_tokens(config: &Config, client: &Arc<ProxiedClient>) -> Arc<HubTokenResolver> {
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    Arc::new(HubTokenResolver::new(
        config.sections.huggingface_hub.clone(),
        config.sections.huggingface_oidc.clone(),
        Arc::clone(client),
        reader,
        clock,
    ))
}

// token counting runs on the request task like LiteLLM's token_counter, so the job runs inline
struct InlineCounter;

#[async_trait]
impl BlockingCounter for InlineCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    // only a fetched tiktoken vocabulary converts, and every encoding the wheel tests count with is bundled
    #[cfg_attr(test, mutants::skip)]
    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

mod tests;

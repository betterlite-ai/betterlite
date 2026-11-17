use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::Settings as HubSettings;
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_core::openai_streaming_assembly::RelayTokenCounter;
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use betterlite_tokenizer::count::settings::Settings;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService, EncodeLimits,
};
use betterlite_tokenizer::engine::selection::Selector;
use tokio::runtime::{Builder, Runtime};
use tokio::time::Instant;

use super::failure;

// The bundled catalog holds every OpenAI encoding, so this fetcher only exists because the selector needs one.
struct InlineCounter;

#[async_trait::async_trait]
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

pub(super) struct BundledCounter {
    counter: Arc<Counter>,
    runtime: Runtime,
}

impl BundledCounter {
    pub(super) fn new() -> Result<Self, Box<HelperError>> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        let service = Arc::new(CountingService::new(
            4,
            Arc::new(InlineCounter),
            EncodeLimits::default(),
        ));
        let policy =
            ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).map_err(failure)?;
        let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy)).map_err(failure)?);
        let fetcher = Arc::new(TiktokenFetcher::new(
            Arc::clone(&client),
            None,
            Arc::clone(&service),
        ));
        let catalog = BundledCatalog::default().get();
        let selector = Arc::new(Selector::new(
            Arc::clone(&fetcher),
            downloader(client),
            Arc::clone(&catalog),
            Arc::clone(&service),
            false,
        ));
        Ok(Self {
            counter: Arc::new(Counter::new(
                selector,
                service,
                catalog,
                Settings::default(),
            )),
            runtime,
        })
    }
}

impl RelayTokenCounter for BundledCounter {
    fn count_messages(&self, model: &str, messages: &[Message]) -> Result<i64, CountError> {
        self.runtime.block_on(UsageTokenCounter::count_messages(
            self.counter.as_ref(),
            model,
            Some(messages),
        ))
    }

    fn count_text(&self, text: &str) -> Result<i64, CountError> {
        self.runtime
            .block_on(UsageTokenCounter::count_text(self.counter.as_ref(), text))
    }

    fn count_response_text(&self, model: &str, text: &str) -> Result<i64, CountError> {
        self.runtime
            .block_on(UsageTokenCounter::count_response_text(
                self.counter.as_ref(),
                model,
                text,
            ))
    }
}

fn downloader(client: Arc<ProxiedClient>) -> Arc<HubDownloader> {
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

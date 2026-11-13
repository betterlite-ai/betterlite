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
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService, EncodeLimits,
};
use betterlite_tokenizer::engine::selection::Selector;
use tokio::time::Instant;

const CATALOG: &str = r#"{"claude-2.1": {"litellm_provider": "anthropic"}}"#;

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

struct FailingCounter;

#[async_trait]
impl BlockingCounter for FailingCounter {
    async fn count(&self, _job: CountJob) -> Result<usize, EngineError> {
        Err(EngineError::PoolClosed)
    }

    async fn build(&self, _job: BuildJob) -> Result<Engine, EngineError> {
        Err(EngineError::PoolClosed)
    }

    async fn convert(&self, _job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        Err(EngineError::PoolClosed)
    }
}

// offline; claude-2.1 uses the bundled Anthropic tokenizer and every other model the bundled cl100k
pub(crate) fn selector() -> Selector {
    selector_with(Arc::new(InlineCounter))
}

// every tokenizer build fails, so selection reports EngineError::PoolClosed
pub(crate) fn failing_selector() -> Selector {
    selector_with(Arc::new(FailingCounter))
}

fn selector_with(facility: Arc<dyn BlockingCounter>) -> Selector {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy)).unwrap());
    let settings = HubSettings {
        endpoint: "https://huggingface.co".to_owned(),
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
    };
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let tokens = Arc::new(HubTokenResolver::new(
        settings.clone(),
        OidcSettings::default(),
        Arc::clone(&client),
        reader,
        clock,
    ));
    let hub = Arc::new(HubDownloader::new(
        Arc::new(settings),
        tokens,
        Arc::new(Instant::now),
    ));
    let counter = Arc::new(CountingService::new(4, facility, EncodeLimits::default()));
    let fetcher = Arc::new(TiktokenFetcher::new(client, None, Arc::clone(&counter)));
    let catalog = Arc::new(ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap());
    Selector::new(fetcher, hub, catalog, counter, false)
}

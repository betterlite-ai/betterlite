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

use crate::router_probes::error::OperationError;

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

// litellm.token_counter over the bundled cl100k vocabulary with the network switched off, the count is_prompt_caching_valid_prompt reads
pub(crate) fn counting_pieces() -> Result<(Counter, Arc<ModelCatalog>), OperationError> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).map_err(invalid)?;
    let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy)).map_err(invalid)?);
    let settings = Settings::default();
    let service = Arc::new(CountingService::new(
        1,
        Arc::new(InlineCounter),
        settings.encode_limits().map_err(invalid)?,
    ));
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
        true,
    ));
    let counter = Counter::new(selector, fetcher, service, Arc::clone(&catalog), settings);
    Ok((counter, catalog))
}

fn downloader(client: Arc<ProxiedClient>) -> Arc<HubDownloader> {
    let hub = HubSettings {
        offline: OfflineSetting::Offline,
        endpoint: "http://127.0.0.1:1".to_owned(),
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: PathBuf::from("unused"),
        hub_cache: PathBuf::from("unused"),
        token_path: PathBuf::from("unused-token"),
        stored_tokens_path: PathBuf::from("unused-stored-tokens"),
        transfer: TransferSetting::Standard,
    };
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let tokens = Arc::new(HubTokenResolver::new(
        hub.clone(),
        OidcSettings::default(),
        client,
        reader,
        Arc::new(SystemHubClock::default()),
    ));
    Arc::new(HubDownloader::new(
        Arc::new(hub),
        tokens,
        Arc::new(Instant::now),
    ))
}

fn invalid<E>(error: E) -> OperationError
where
    E: ToString,
{
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}

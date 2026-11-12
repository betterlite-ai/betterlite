use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;

use crate::count::Counter;
use crate::count::settings::Settings;
use crate::engine::Engine;
use crate::engine::error::EngineError;
use crate::engine::fetch::TiktokenFetcher;
use crate::engine::hub::HubDownloader;
use crate::engine::limits::{BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService};
use crate::engine::selection::Selector;
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

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

pub(super) fn counter() -> Counter {
    build_counter(Arc::new(InlineCounter))
}

pub(super) fn failing_counter() -> Counter {
    build_counter(Arc::new(FailingCounter))
}

fn build_counter(facility: Arc<dyn BlockingCounter>) -> Counter {
    let settings = Settings {
        disabled: false,
        max_exact_chars: NonZeroUsize::new(4_000_000).expect("the limit is positive"),
        chunk_chars: NonZeroUsize::new(1024).expect("the chunk is positive"),
    };
    let client = client();
    let service = Arc::new(CountingService::new(
        4,
        facility,
        settings.encode_limits().expect("the limits fit"),
    ));
    let fetcher = Arc::new(TiktokenFetcher::new(
        Arc::clone(&client),
        None,
        Arc::clone(&service),
    ));
    let hub = hub_settings(
        "http://127.0.0.1:1",
        Path::new("/nonexistent/betterlite-b4802"),
    );
    let downloader = Arc::new(HubDownloader::new(
        Arc::new(hub.clone()),
        resolver(&hub, &client),
        Arc::new(Instant::now),
    ));
    let catalog = catalog();
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        downloader,
        Arc::clone(&catalog),
        Arc::clone(&service),
        false,
    ));
    Counter::new(selector, fetcher, service, catalog, settings)
}

fn client() -> Arc<ProxiedClient> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the upstream policy resolves"),
    );
    Arc::new(ProxiedClient::outbound(&policy).expect("the client builds"))
}

fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

fn hub_settings(endpoint: &str, directory: &Path) -> HubSettings {
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

fn resolver(settings: &HubSettings, client: &Arc<ProxiedClient>) -> Arc<HubTokenResolver> {
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
        Arc::clone(client),
        reader,
        clock,
    ))
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
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
use betterlite_models::catalog::BundledCatalog;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::request::{TextInput, TokenRequest};
use betterlite_tokenizer::count::settings::Settings;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService, EncodeLimits,
};
use betterlite_tokenizer::engine::selection::Selector;
use tokio::runtime::Builder;
use tokio::time::Instant;

use crate::agentcore_response::counter::TokenCounter;

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

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn hub_settings() -> HubSettings {
    HubSettings {
        endpoint: "http://127.0.0.1:1".to_owned(),
        offline: OfflineSetting::Online,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: PathBuf::from("unused"),
        hub_cache: PathBuf::from("unused"),
        token_path: PathBuf::from("unused-token"),
        stored_tokens_path: PathBuf::from("unused-stored-tokens"),
        transfer: TransferSetting::Standard,
    }
}

fn counter() -> Counter {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy)).unwrap());
    let service = Arc::new(CountingService::new(
        4,
        Arc::new(InlineCounter),
        EncodeLimits::default(),
    ));
    let fetcher = Arc::new(TiktokenFetcher::new(
        Arc::clone(&client),
        None,
        Arc::clone(&service),
    ));
    let catalog = BundledCatalog::default().get();
    let hub = hub_settings();
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let resolver = Arc::new(HubTokenResolver::new(
        hub.clone(),
        OidcSettings::default(),
        Arc::clone(&client),
        reader,
        clock,
    ));
    let downloader = Arc::new(HubDownloader::new(
        Arc::new(hub),
        resolver,
        Arc::new(Instant::now),
    ));
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        downloader,
        Arc::clone(&catalog),
        Arc::clone(&service),
        false,
    ));
    Counter::new(selector, fetcher, service, catalog, Settings::default())
}

#[test]
fn token_counter_impl_forwards_to_b196() {
    let token_counter = counter();
    let text = TextInput::One("hello world".to_owned());
    let request = TokenRequest {
        model: "gpt-4",
        custom_tokenizer: None,
        text: Some(&text),
        messages: None,
        count_response_tokens: Some(true),
        tools: None,
        tool_choice: None,
    };
    let counted = block_on(TokenCounter::count(&token_counter, &request)).unwrap();
    assert!(counted > 1, "{counted}");
}

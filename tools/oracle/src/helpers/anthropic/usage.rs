use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_anthropic::usage::{
    AnthropicUsage, ReasoningTokenCounter, calculate_usage, is_anthropic_usage_object,
};
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
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{TextInput, TokenRequest};
use betterlite_tokenizer::count::settings::Settings as CountingSettings;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::{Clock, HubDownloader};
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService,
};
use betterlite_tokenizer::engine::selection::Selector;
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};
use tokio::time::Instant;

use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    usage: Json,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    completion_response: Option<Json>,
    #[serde(default)]
    speed: Option<String>,
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

struct LocalCounter {
    counter: Counter,
    runtime: Runtime,
}

impl ReasoningTokenCounter for LocalCounter {
    type Failure = CountError;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, Self::Failure> {
        let input = TextInput::One(text.to_owned());
        let request = TokenRequest {
            model: "",
            custom_tokenizer: None,
            text: Some(&input),
            messages: None,
            count_response_tokens: Some(true),
            tools: None,
            tool_choice: None,
        };
        self.runtime.block_on(self.counter.count(&request))
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let failure = |message: String| {
            Box::new(HelperError {
                class: "AnthropicUsage".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            })
        };
        let counter = local_counter().map_err(failure)?;
        let usage = calculate_usage(
            &input.usage,
            input.reasoning_content.as_deref(),
            input.completion_response.as_ref(),
            input.speed.as_deref(),
            &counter,
        )
        .map_err(|error| failure(error.to_string()))?;
        serde_json::to_value(UsageAnswer {
            usage,
            is_anthropic_usage_object: is_anthropic_usage_object(&input.usage),
        })
        .map_err(|error| failure(error.to_string()))
    }
}

fn local_counter() -> Result<LocalCounter, String> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| error.to_string())?,
    );
    let client = Arc::new(ProxiedClient::outbound(&policy).map_err(|error| error.to_string())?);
    let settings = CountingSettings::default();
    let limits = settings
        .encode_limits()
        .map_err(|error| error.to_string())?;
    let service = Arc::new(CountingService::new(4, Arc::new(InlineCounter), limits));
    let fetcher = Arc::new(TiktokenFetcher::new(
        Arc::clone(&client),
        None,
        Arc::clone(&service),
    ));
    let catalog = BundledCatalog::default().get();
    let download_clock: Arc<Clock> = Arc::new(Instant::now);
    let hub = Arc::new(HubDownloader::new(
        Arc::new(hub_settings()),
        hub_tokens(client),
        download_clock,
    ));
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        hub,
        Arc::clone(&catalog),
        Arc::clone(&service),
        true,
    ));
    let counter = Counter::new(selector, fetcher, service, catalog, settings);
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    Ok(LocalCounter { counter, runtime })
}

fn hub_settings() -> HubSettings {
    HubSettings {
        endpoint: "https://huggingface.co".to_owned(),
        offline: OfflineSetting::Offline,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: true,
        env_token: None,
        hf_home: PathBuf::new(),
        hub_cache: PathBuf::new(),
        token_path: PathBuf::new(),
        stored_tokens_path: PathBuf::new(),
        transfer: TransferSetting::Standard,
    }
}

fn hub_tokens(client: Arc<ProxiedClient>) -> Arc<HubTokenResolver> {
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
        hub_settings(),
        oidc,
        client,
        reader,
        clock,
    ))
}

#[derive(Serialize)]
struct UsageAnswer {
    usage: AnthropicUsage,
    is_anthropic_usage_object: bool,
}

mod tests;

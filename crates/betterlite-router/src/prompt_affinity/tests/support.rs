use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_core::message_utilities::prompt_cache_checks::{
    PromptCacheSettings, get_prompt_cache_min_tokens,
};
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_metadata::base64::Base64Truncator;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_prompts::cache_accounting::dialect::ModelFacts;
use betterlite_prompts::cache_defaults::CacheFacts;
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::settings::Settings as TokenizerSettings;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService, EncodeLimits,
};
use betterlite_tokenizer::engine::selection::Selector;
use indexmap::IndexMap;
use tokio::time::Instant;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::prompt_affinity::cache::PinCache;
use crate::prompt_affinity::prefix::{
    PrefixPosition, lookback_keys, prefix_positions, prompt_caching_cache_key,
};
use crate::prompt_affinity::service::{
    FilterRequest, InjectionDefaults, PromptAffinity, PromptAffinityDependencies, SuccessEvent,
    TokenGate,
};

pub(crate) const MARKED_MESSAGES: &str = r#"[{"role":"system","content":"be brief"},{"role":"user","content":"why is the sky blue","cache_control":{"type":"ephemeral"}}]"#;

const CATALOG: &str = r#"{
  "claude-min": {"litellm_provider": "anthropic", "supports_prompt_caching": true, "prompt_cache_min_tokens": 400},
  "claude-plain": {"litellm_provider": "anthropic", "supports_prompt_caching": true}
}"#;

pub(super) fn catalog() -> Arc<ModelCatalog> {
    Arc::new(ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Warn).unwrap())
}

pub(super) fn truncator() -> Base64Truncator {
    Base64Truncator::new(64).unwrap()
}

pub(crate) fn messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn json_messages(source: &str) -> Vec<Json> {
    serde_json::from_str(source).unwrap()
}

pub(super) fn request_kwargs(source: &str) -> betterlite_prompts::arguments::json::Object {
    serde_json::from_str(source).unwrap()
}

pub(super) fn deployment(id: &str, model: &str) -> DeploymentSpec {
    let mut params = IndexMap::new();
    params.insert("model".to_owned(), Json::Text(model.to_owned()));
    let mut info = IndexMap::new();
    info.insert("id".to_owned(), Json::Text(id.to_owned()));
    DeploymentSpec::new("group".to_owned(), params, info, IndexMap::new(), || {
        "generated".to_owned()
    })
    .unwrap()
}

pub(crate) fn pin(model_id: &str) -> Json {
    let mut fields = IndexMap::new();
    fields.insert("model_id".to_owned(), Json::Text(model_id.to_owned()));
    Json::Object(fields)
}

#[derive(Default)]
pub(crate) struct FakePins {
    values: Mutex<BTreeMap<String, Json>>,
}

impl FakePins {
    pub(crate) fn seed(&self, key: &str, value: Json) {
        self.values.lock().unwrap().insert(key.to_owned(), value);
    }

    pub(super) fn keys(&self) -> Vec<String> {
        self.values.lock().unwrap().keys().cloned().collect()
    }
}

#[async_trait]
impl PinCache for FakePins {
    async fn store(&self, key: &str, value: &Json) {
        self.seed(key, value.clone());
    }

    async fn read(&self, keys: &[String]) -> Vec<Option<Json>> {
        let values = self.values.lock().unwrap();
        keys.iter().map(|key| values.get(key).cloned()).collect()
    }
}

pub(super) struct FakeGate {
    pub(super) counted: u64,
    pub(super) settings: PromptCacheSettings,
    pub(super) catalog: Arc<ModelCatalog>,
}

#[async_trait]
impl TokenGate for FakeGate {
    async fn meets_minimum(&self, model: &str, _messages: &[Json], minimum: Option<u64>) -> bool {
        let threshold = minimum
            .unwrap_or_else(|| get_prompt_cache_min_tokens(model, self.settings, &self.catalog));
        self.counted >= threshold
    }
}

pub(super) struct ClaudeFacts;

impl ModelFacts for ClaudeFacts {
    fn cache_breakpoint_flag(&self, _model: &str) -> Option<bool> {
        None
    }

    fn provider(&self, model: &str) -> Option<String> {
        if model.contains("claude") {
            Some("anthropic".to_owned())
        } else {
            None
        }
    }
}

impl CacheFacts for ClaudeFacts {
    fn supports_prompt_caching(&self, model: &str, provider: &str) -> bool {
        provider == "anthropic" && model.contains("claude")
    }
}

pub(crate) fn service(pins: Arc<FakePins>, counted: u64, global_enabled: bool) -> PromptAffinity {
    let catalog = catalog();
    PromptAffinity::from(PromptAffinityDependencies {
        pins,
        gate: Arc::new(FakeGate {
            counted,
            settings: PromptCacheSettings::default(),
            catalog: Arc::clone(&catalog),
        }),
        facts: Arc::new(ClaudeFacts),
        catalog,
        settings: PromptCacheSettings::default(),
        truncator: truncator(),
        defaults: InjectionDefaults {
            global_enabled,
            ttl: None,
            alias_map: BTreeMap::new(),
        },
    })
}

pub(super) fn request<'pool>(
    pool: &'pool [DeploymentSpec],
    messages: Option<&'pool [Message]>,
) -> FilterRequest<'pool> {
    FilterRequest {
        model: "group",
        deployments: pool,
        messages,
        request_kwargs: None,
        priority_group: false,
        target_order: None,
    }
}

pub(super) fn success<'event>(
    call_type: &'event str,
    model_id: Option<&'event str>,
    messages: Option<&'event [Json]>,
) -> SuccessEvent<'event> {
    SuccessEvent {
        call_type,
        model: "group",
        messages,
        model_id,
    }
}

pub(super) fn ids(deployments: &[DeploymentSpec]) -> Vec<&str> {
    deployments
        .iter()
        .filter_map(|deployment| deployment.id())
        .collect()
}

pub(crate) fn cache_key(messages: &[Json], tools: Option<&[Json]>) -> Option<String> {
    prompt_caching_cache_key(Some(messages), tools, &truncator()).unwrap()
}

pub(super) fn positions(messages: &[Json]) -> Vec<PrefixPosition> {
    prefix_positions(Some(messages), None, &truncator()).unwrap()
}

pub(super) fn lookback(messages: &[Json]) -> Vec<String> {
    lookback_keys(&positions(messages))
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

// the offline token counter the io_token_reservations tests use, so the gate counts like production
pub(super) fn token_counter() -> Counter {
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
    let hub = HubSettings {
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
    };
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
    Counter::new(
        selector,
        fetcher,
        service,
        catalog,
        TokenizerSettings::default(),
    )
}

pub(super) fn text_blocks_source(count: usize) -> String {
    let mut blocks: Vec<String> = (0..count.saturating_sub(1))
        .map(|index| format!(r#"{{"type":"text","text":"block {index}"}}"#))
        .collect();
    blocks.push(r#"{"type":"text","text":"last","cache_control":{"type":"ephemeral"}}"#.to_owned());
    format!(r#"[{{"role":"user","content":[{}]}}]"#, blocks.join(","))
}

pub(super) fn tool_result_blocks_source(count: usize) -> String {
    let mut blocks = Vec::new();
    for index in 0..count.saturating_sub(1) {
        let id = format!("call {index}");
        let value = format!("value {index}");
        blocks.push(format!(
            r#"{{"type":"tool_result","tool_use_id":"{id}","content":"{value}"}}"#
        ));
    }
    blocks.push(
        r#"{"type":"tool_result","tool_use_id":"last","content":"last","cache_control":{"type":"ephemeral"}}"#
            .to_owned(),
    );
    format!(r#"[{{"role":"tool","content":[{}]}}]"#, blocks.join(","))
}

pub(super) fn image_message_source(payload: &str) -> String {
    format!(
        r#"[{{"role":"user","content":[{{"type":"image_url","image_url":{{"url":"data:image/png;base64,{payload}"}}}},{{"type":"text","text":"last","cache_control":{{"type":"ephemeral"}}}}]}}]"#
    )
}

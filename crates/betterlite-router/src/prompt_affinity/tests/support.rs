use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_core::message_utilities::prompt_cache_checks::{
    PromptCacheSettings, get_prompt_cache_min_tokens,
};
use betterlite_logging::logging_metadata::base64::Base64Truncator;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_prompts::cache_accounting::dialect::ModelFacts;
use betterlite_prompts::cache_defaults::CacheFacts;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::prompt_affinity::cache::PinCache;
use crate::prompt_affinity::prefix::{
    PrefixPosition, lookback_keys, prefix_positions, prompt_caching_cache_key,
};
use crate::prompt_affinity::service::{
    FilterRequest, InjectionDefaults, PromptAffinity, PromptAffinityDependencies, SuccessEvent,
    TokenGate,
};

pub(super) const MARKED_MESSAGES: &str = r#"[{"role":"system","content":"be brief"},{"role":"user","content":"why is the sky blue","cache_control":{"type":"ephemeral"}}]"#;

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

pub(super) fn messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).unwrap()
}

pub(super) fn json_messages(source: &str) -> Vec<Json> {
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
    DeploymentSpec::new(
        "group".to_owned(),
        params,
        info,
        IndexMap::new(),
        || "generated".to_owned(),
    )
    .unwrap()
}

pub(super) fn pin(model_id: &str) -> Json {
    let mut fields = IndexMap::new();
    fields.insert("model_id".to_owned(), Json::Text(model_id.to_owned()));
    Json::Object(fields)
}

#[derive(Default)]
pub(super) struct FakePins {
    values: Mutex<BTreeMap<String, Json>>,
}

impl FakePins {
    pub(super) fn seed(&self, key: &str, value: Json) {
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

pub(super) fn service(
    pins: Arc<FakePins>,
    counted: u64,
    global_enabled: bool,
) -> PromptAffinity {
    let catalog = catalog();
    PromptAffinity::new(PromptAffinityDependencies {
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

pub(super) fn cache_key(messages: &[Json], tools: Option<&[Json]>) -> Option<String> {
    prompt_caching_cache_key(Some(messages), tools, &truncator()).unwrap()
}

pub(super) fn positions(messages: &[Json]) -> Vec<PrefixPosition> {
    prefix_positions(Some(messages), None, &truncator()).unwrap()
}

pub(super) fn lookback(messages: &[Json]) -> Vec<String> {
    lookback_keys(&positions(messages))
}

pub(super) fn text_blocks_source(count: usize) -> String {
    let mut blocks = Vec::new();
    for index in 0..count.saturating_sub(1) {
        blocks.push(format!(r#"{{"type":"text","text":"block {index}"}}"#));
    }
    blocks.push(r#"{"type":"text","text":"last","cache_control":{"type":"ephemeral"}}"#.to_owned());
    format!(r#"[{{"role":"user","content":[{}]}}]"#, blocks.join(","))
}

pub(super) fn tool_result_blocks_source(count: usize) -> String {
    let mut blocks = Vec::new();
    for index in 0..count.saturating_sub(1) {
        blocks.push(format!(
            r#"{{"type":"tool_result","tool_use_id":"call {index}","content":"value {index}"}}"#
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

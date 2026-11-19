use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::future::Future;
use std::future::ready;
use std::sync::Mutex;
use std::sync::PoisonError;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::cache_observer::ObservationCache;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::price_lookup::PricedModel;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::tiered_pricing::Scalar;
use chrono::DateTime;
use chrono::Utc;
use http::HeaderMap;
use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::cache_aware_routing::services::CacheCostService;
use crate::cache_aware_routing::services::MinimumCacheTokens;
use crate::cache_aware_routing::services::PredictionServices;
use crate::cache_aware_routing::services::PriceRequest;
use crate::cache_aware_routing::services::ScenarioRequest;
use crate::cache_aware_routing::types::CacheCostScenario;
use crate::cache_aware_routing::types::CacheTokenBuckets;
use crate::cache_prediction_targets::counting::TokenCounter;
use crate::cache_prediction_targets::scope::cache_scope;
use crate::count_tokens::handler::CountTokensTransport;
use crate::credentials::ANTHROPIC_VERSION;
use crate::credentials::AnthropicSettings;

pub(crate) const MODEL: &str = "claude-x";
pub(crate) const KEY: &str = "sk-ant-api03-x";
pub(crate) const CALLER: &str = "caller";
pub(crate) const DEPLOYMENT_ID: &str = "deployment-a";
pub(crate) const PARAMS: &str = r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x"}"#;
pub(crate) const BODY: &str = r#"{"model":"claude-x","max_tokens":64,"messages":[{"role":"user","content":[{"type":"text","text":"hello","cache_control":{"type":"ephemeral"}}]}]}"#;
pub(crate) const MULTI_BODY: &str = r#"{"model":"claude-x","max_tokens":64,"messages":[{"role":"user","content":"one"},{"role":"assistant","content":"two"},{"role":"user","content":[{"type":"text","text":"three","cache_control":{"type":"ephemeral"}}]}]}"#;
pub(crate) const ONE_HOUR_BODY: &str = r#"{"model":"claude-x","max_tokens":64,"messages":[{"role":"user","content":[{"type":"text","text":"hello","cache_control":{"type":"ephemeral","ttl":"1h"}}]}]}"#;

const BILLING_SECONDS: i64 = 1_700_000_000;

pub(crate) fn billing_time() -> DateTime<Utc> {
    DateTime::from_timestamp(BILLING_SECONDS, 0).expect("a valid instant")
}

pub(crate) fn settings(api_base: &str) -> AnthropicSettings {
    AnthropicSettings {
        api_key: None,
        auth_token: None,
        api_base: api_base.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}

pub(crate) fn params(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("deployment params")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("json")
}

pub(crate) fn model_info() -> JsonObject {
    params(r#"{"id":"deployment-a","max_input_tokens":200000,"max_output_tokens":64000}"#)
}

pub(crate) fn scope(deployment_id: &str, model: &str) -> String {
    cache_scope(CALLER, deployment_id, KEY, model)
}

// cache_observer's key layout, which a test must match to seed the observation predict looks up.
pub(crate) fn observation_key(scope: &str, fingerprint: &str) -> String {
    format!("prompt-cache-observation:{scope}:{fingerprint}")
}

pub(crate) fn observation_json(
    fingerprint: &str,
    cached_tokens: i64,
    observed_at: f64,
    expires_at: f64,
) -> Json {
    let text = format!(
        r#"{{"fingerprint":"{fingerprint}","cached_tokens":{cached_tokens},"observed_at":{observed_at},"expires_at":{expires_at}}}"#
    );
    json(&text)
}

pub(crate) struct Resolution {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Resolution {
    pub(crate) fn new() -> Self {
        Self {
            catalog: ModelCatalog::default(),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    pub(crate) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

pub(crate) struct MemoryCache {
    entries: Mutex<BTreeMap<String, Json>>,
}

impl MemoryCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn seed(&self, key: &str, value: Json) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }
}

#[async_trait::async_trait]
impl ObservationCache for MemoryCache {
    async fn get(&self, key: &str) -> Option<Json> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    async fn set(&self, key: &str, value: &Json, _ttl_seconds: f64, _local_only: bool) {
        self.seed(key, value.clone());
    }

    fn has_redis(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub(crate) struct ScenarioCall {
    pub(crate) model: String,
    pub(crate) deployment_id: String,
    pub(crate) tokens: CacheTokenBuckets,
    pub(crate) moment: DateTime<Utc>,
}

#[derive(Clone)]
pub(crate) struct PriceCall {
    pub(crate) model: String,
    pub(crate) deployment_id: String,
    pub(crate) tokens: CacheTokenBuckets,
    pub(crate) completion_tokens: i64,
    pub(crate) moment: DateTime<Utc>,
}

impl PriceCall {
    fn new(request: &PriceRequest<'_>, moment: DateTime<Utc>) -> Self {
        Self {
            model: request.model.to_owned(),
            deployment_id: request.deployment_id.to_owned(),
            tokens: *request.tokens,
            completion_tokens: request.completion_tokens,
            moment,
        }
    }
}

pub(crate) struct FakeCost {
    pub(crate) cost: Option<Decimal>,
    pub(crate) calls: Mutex<Vec<ScenarioCall>>,
    pub(crate) prices: Mutex<Vec<PriceCall>>,
}

impl FakeCost {
    pub(crate) fn new(cost: Option<Decimal>) -> Self {
        Self {
            cost,
            calls: Mutex::new(Vec::new()),
            prices: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<ScenarioCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn prices(&self) -> Vec<PriceCall> {
        self.prices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CacheCostService for FakeCost {
    fn buckets(
        &self,
        prefix_tokens: i64,
        suffix_tokens: i64,
        read_tokens: i64,
        ttl_seconds: u32,
    ) -> CacheTokenBuckets {
        let creation = prefix_tokens.saturating_sub(read_tokens).max(0);
        CacheTokenBuckets {
            uncached_input_tokens: suffix_tokens,
            cache_read_input_tokens: read_tokens,
            cache_creation_5m_input_tokens: if ttl_seconds == 300 { creation } else { 0 },
            cache_creation_1h_input_tokens: if ttl_seconds == 3600 { creation } else { 0 },
        }
    }

    fn scenario(
        &self,
        request: &ScenarioRequest<'_>,
        moment: DateTime<Utc>,
    ) -> Option<CacheCostScenario> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ScenarioCall {
                model: request.model.to_owned(),
                deployment_id: request.deployment_id.to_owned(),
                tokens: *request.tokens,
                moment,
            });
        let cost = self.cost?;
        Some(CacheCostScenario {
            tokens: *request.tokens,
            input_cost: cost,
        })
    }

    fn price(&self, request: &PriceRequest<'_>, moment: DateTime<Utc>) -> Option<Decimal> {
        self.prices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(PriceCall::new(request, moment));
        self.cost
    }
}

pub(crate) struct FakeMinimum {
    pub(crate) tokens: i64,
}

impl MinimumCacheTokens for FakeMinimum {
    fn minimum_tokens(&self, _model: &str) -> i64 {
        self.tokens
    }
}

// The minimum service that also reports which model name the prediction asked it about.
pub(crate) struct RecordingMinimum {
    pub(crate) tokens: i64,
    asked: Mutex<Vec<String>>,
}

impl RecordingMinimum {
    pub(crate) fn new(tokens: i64) -> Self {
        Self {
            tokens,
            asked: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl MinimumCacheTokens for RecordingMinimum {
    fn minimum_tokens(&self, model: &str) -> i64 {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        self.tokens
    }
}

pub(crate) struct ScriptedCounter {
    replies: Mutex<VecDeque<Option<i64>>>,
    asked: Mutex<Vec<Json>>,
}

impl ScriptedCounter {
    pub(crate) fn new(replies: &[Option<i64>]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().copied().collect()),
            asked: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn asked(&self) -> Vec<Json> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TokenCounter for ScriptedCounter {
    fn count(
        &self,
        _model: &str,
        _api_key: &SecretString,
        body: &Json,
    ) -> impl Future<Output = Option<i64>> + Send {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(body.clone());
        ready(
            self.replies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .flatten(),
        )
    }
}

pub(crate) struct LimitsLookup {
    pub(crate) max_input: Option<i64>,
    pub(crate) max_output: Option<i64>,
}

impl PriceLookup for LimitsLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        let mut record = PriceRecord::new();
        if let Some(limit) = self.max_input {
            record.insert(
                "max_input_tokens".to_owned(),
                Scalar::Number(serde_json::Number::from(limit)),
            );
        }
        if let Some(limit) = self.max_output {
            record.insert(
                "max_output_tokens".to_owned(),
                Scalar::Number(serde_json::Number::from(limit)),
            );
        }
        Some(record)
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

#[derive(Clone)]
pub(crate) struct SentCount {
    pub(crate) url: String,
    pub(crate) api_key: Option<String>,
}

pub(crate) struct StubTransport {
    pub(crate) status: u16,
    pub(crate) payload: Vec<u8>,
    pub(crate) sent: Mutex<Vec<SentCount>>,
}

impl StubTransport {
    pub(crate) fn replying(status: u16, payload: &str) -> Self {
        Self {
            status,
            payload: payload.as_bytes().to_vec(),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn request(&self) -> SentCount {
        let mut sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        sent.remove(0)
    }

    pub(crate) fn requests(&self) -> Vec<SentCount> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CountTokensTransport for StubTransport {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let api_key = request
            .headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SentCount {
                url: request.url.to_string(),
                api_key,
            });
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.payload.clone(),
        }))
    }
}

pub(crate) fn prediction_services<'services>(
    settings: &'services AnthropicSettings,
    resolution: &'services ResolutionContext<'services>,
    cache: &'services dyn ObservationCache,
    cost: &'services dyn CacheCostService,
    minimum: &'services dyn MinimumCacheTokens,
) -> PredictionServices<'services> {
    PredictionServices {
        settings,
        resolution,
        cache,
        cost,
        minimum,
        billing_time: billing_time(),
    }
}

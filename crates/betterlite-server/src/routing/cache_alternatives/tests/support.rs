use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::{Future, pending, ready};
use std::str::{FromStr as _, from_utf8};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_anthropic::cache_aware_routing::services::{
    CacheCostService, MinimumCacheTokens, PredictionServices, PriceRequest, ScenarioRequest,
};
use betterlite_anthropic::cache_aware_routing::types::{CacheCostScenario, CacheTokenBuckets};
use betterlite_anthropic::cache_plan::parse_prompt;
use betterlite_anthropic::cache_prediction_targets::scope::cache_scope;
use betterlite_anthropic::count_tokens::handler::{
    AnthropicCountTokensHandler, CountTokensTransport,
};
use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::entity_model_access::error::EntityModelAccessError;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::anthropic_count_tokens::DEFAULT_BETA_VERSION;
use betterlite_config::sections::anthropic_credentials::DEFAULT_API_BASE;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::cache_observer::ObservationCache;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_prompts::message_content::Message;
use betterlite_rate_limit::pre_call_hook::PreCallHookError;
use betterlite_rate_limit::windowed_admission::WindowedAdmissionError;
use betterlite_router::cache_cost_routing::{PreRoutingResponse, TierParams};
use betterlite_router::complexity_config::checks::Feature;
use betterlite_router::complexity_scoring::decision::RoutingDecision;
use betterlite_router::deployment_catalog::error::CatalogLookupError;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::router_core::error::RoutingError;
use betterlite_router::router_predicates::JsonMap;
use betterlite_router::tier_definitions::model::{TierModelEntry, TierPool, TierSet};
use chrono::{DateTime, Utc};
use http::HeaderMap;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::routing::cache_alternatives::config::CacheRoutingConfig;
use crate::routing::cache_alternatives::host::{CacheRoutingHost, CapacityLimiter, CapacitySlot};

pub(super) const CALLER: &str = "test-cache-aware-caller";
pub(super) const PROVIDER_KEY: &str = "test-cache-aware-provider";
pub(super) const NOW: f64 = 1000.0;
pub(super) const CHEAP_MODEL: &str = "claude-haiku-4-5";
pub(super) const STRONG_MODEL: &str = "claude-sonnet-5";
pub(super) const CHEAP_ID: &str = "test-cache-cheap";
pub(super) const STRONG_ID: &str = "test-cache-strong";
pub(super) const COLD_ID: &str = "test-cache-cold";
pub(super) const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
pub(super) const UNAVAILABLE_LOG: &str =
    "Cache-aware routing unavailable; keeping the classified model";

const BILLING_SECONDS: i64 = 1_700_000_000;

pub(super) fn billing_time() -> DateTime<Utc> {
    DateTime::from_timestamp(BILLING_SECONDS, 0).expect("a valid instant")
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON literal")
}

pub(super) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("a JSON object")
}

pub(super) fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

// LiteLLM's _body: one cache breakpoint over the stable context, then the question.
pub(super) fn body(text: &str) -> Json {
    let rendered = format!(
        r#"{{"max_tokens":20000,"messages":[{{"role":"user","content":[{{"type":"text","text":"{text}","cache_control":{{"type":"ephemeral"}}}},{{"type":"text","text":"What is 2 + 2?"}}]}}]}}"#
    );
    json(&rendered)
}

pub(super) fn body_with_max_tokens(text: &str, max_tokens: i64) -> Json {
    let mut body = body(text);
    let Json::Object(fields) = &mut body else {
        panic!("the body is an object");
    };
    fields.insert("max_tokens".to_owned(), number(max_tokens));
    body
}

pub(super) fn messages(body: &Json) -> Vec<Message> {
    let Json::Object(fields) = body else {
        panic!("the body is an object");
    };
    let Some(Json::List(items)) = fields.get("messages") else {
        panic!("the body holds messages");
    };
    let text = serde_json::to_string(items).expect("the messages serialize");
    serde_json::from_str(&text).expect("the messages are typed messages")
}

pub(super) fn decision(entries: &[(&str, &str)]) -> RoutingDecision {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), Json::Text((*value).to_owned())))
        .collect()
}

pub(super) fn response(tier: &str, model: &str) -> PreRoutingResponse {
    PreRoutingResponse {
        model: model.to_owned(),
        messages: None,
        routing_decision: Some(decision(&[
            ("router_model_name", "smart"),
            ("router_type", "complexity"),
            ("routed_model", model),
            ("tier", tier),
            ("cause", "heuristic_scorer"),
        ])),
        session_affinity_ttl_seconds: None,
        litellm_params: None,
    }
}

pub(super) fn ladder(entries: &[(&str, &str)]) -> TierSet {
    TierSet {
        definitions: None,
        pools: entries
            .iter()
            .map(|(tier, model)| {
                (
                    (*tier).to_owned(),
                    TierPool::Single(TierModelEntry {
                        model_name: (*model).to_owned(),
                        params: IndexMap::new(),
                    }),
                )
            })
            .collect(),
        labels: BTreeMap::new(),
        non_reasoning: false,
        fallback: None,
    }
}

pub(super) fn caller(api_key: &str) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert("api_key".to_owned(), JsonData::String(api_key.to_owned()));
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the caller decodes")
}

// The key hash a caller's observations live under: the proxy stores the hashed api key.
pub(super) fn caller_key(api_key: &str) -> String {
    let auth = caller(api_key);
    account_limits(&auth)
        .hashed_api_key
        .expect("the caller's key hashed")
        .to_owned()
}

pub(super) fn request_kwargs(api_key: &str, body: &Json) -> JsonMap {
    let caller_fields = JsonObject::from([("api_key".to_owned(), Json::Text(api_key.to_owned()))]);
    let metadata =
        JsonObject::from([("user_api_key_auth".to_owned(), Json::Object(caller_fields))]);
    let proxy = JsonObject::from([
        ("url".to_owned(), Json::Text(MESSAGES_URL.to_owned())),
        ("body".to_owned(), body.clone()),
        ("headers".to_owned(), Json::Object(JsonObject::new())),
    ]);
    JsonMap::from([
        ("litellm_metadata".to_owned(), Json::Object(metadata)),
        ("proxy_server_request".to_owned(), Json::Object(proxy)),
    ])
}

// One seeded prompt-cache observation for the strong deployment.
pub(super) struct Observation<'observation> {
    pub(super) deployment: &'observation str,
    pub(super) model: &'observation str,
    pub(super) body: &'observation Json,
    pub(super) tokens: i64,
    pub(super) observed_at: f64,
    pub(super) expires_at: f64,
}

pub(super) fn seed_observation(
    cache: &MemoryCache,
    caller_hash: &str,
    observation: &Observation<'_>,
) {
    let text = serde_json::to_string(observation.body).expect("the body serializes");
    let prefix = parse_prompt(&text).expect("the body has one cache breakpoint");
    let scope = cache_scope(
        caller_hash,
        observation.deployment,
        PROVIDER_KEY,
        observation.model,
    );
    let value = json(&format!(
        r#"{{"fingerprint":"{}","cached_tokens":{},"observed_at":{},"expires_at":{}}}"#,
        prefix.fingerprint, observation.tokens, observation.observed_at, observation.expires_at
    ));
    cache.seed(
        &format!("prompt-cache-observation:{scope}:{}", prefix.fingerprint),
        value,
    );
}

// The LiteLLM fixtures' observation: the strong deployment, 50000 cached tokens, expiring at 1290.
pub(super) fn seed(cache: &MemoryCache, body: &Json, expires_at: f64) {
    let caller_hash = caller_key(CALLER);
    seed_observation(
        cache,
        &caller_hash,
        &Observation {
            deployment: STRONG_ID,
            model: STRONG_MODEL,
            body,
            tokens: 50_000,
            observed_at: 990.0,
            expires_at,
        },
    );
}

#[derive(Default)]
pub(super) struct Resolution {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Resolution {
    pub(super) fn context(&self) -> ResolutionContext<'_> {
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

#[derive(Default)]
pub(super) struct MemoryCache {
    entries: Mutex<BTreeMap<String, Json>>,
}

impl MemoryCache {
    pub(super) fn seed(&self, key: &str, value: Json) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }
}

#[async_trait]
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

// The per-token prices the LiteLLM fixtures declare for the cheap and strong deployments.
#[derive(Clone, Copy, Debug)]
pub(super) struct Rates {
    pub(super) input: Decimal,
    pub(super) output: Decimal,
    pub(super) read: Decimal,
    pub(super) creation: Decimal,
}

impl Rates {
    pub(super) fn cheap() -> Self {
        Self::from_prices(&CHEAP_PRICES)
    }

    pub(super) fn strong() -> Self {
        Self::from_prices(&STRONG_PRICES)
    }

    // the same prices the deployments declare, so the cost the arm reports matches its own rates
    fn from_prices(prices: &DeploymentPrices) -> Self {
        Self {
            input: decimal(prices.input),
            output: decimal(prices.output),
            read: decimal(prices.read),
            creation: decimal(prices.creation),
        }
    }
}

// the fixture rates and token counts keep every product far inside Decimal's range
fn tokens_cost(rates: Rates, tokens: &CacheTokenBuckets) -> Decimal {
    let parts = [
        (rates.input, tokens.uncached_input_tokens),
        (rates.read, tokens.cache_read_input_tokens),
        (rates.creation, tokens.cache_creation_5m_input_tokens),
        (rates.creation, tokens.cache_creation_1h_input_tokens),
    ];
    let mut total = Decimal::ZERO;
    for (rate, count) in parts {
        let cost = rate
            .checked_mul(Decimal::from(count))
            .expect("the fixture rates stay in range");
        total = total
            .checked_add(cost)
            .expect("the fixture costs stay in range");
    }
    total
}

// price_cache_tokens over the fixtures' rates: the arm's own buckets plus the completion the caller prices.
pub(super) struct RatesCost {
    table: Mutex<BTreeMap<String, Rates>>,
}

impl Default for RatesCost {
    fn default() -> Self {
        let table = BTreeMap::from([
            (CHEAP_MODEL.to_owned(), Rates::cheap()),
            (STRONG_MODEL.to_owned(), Rates::strong()),
        ]);
        Self {
            table: Mutex::new(table),
        }
    }
}

impl RatesCost {
    pub(super) fn set(&self, model: &str, rates: Rates) {
        self.table
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(model.to_owned(), rates);
    }

    fn rates_for(&self, model: &str) -> Option<Rates> {
        self.table
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(model)
            .copied()
    }

    // the fixture's rates and the arm's own buckets priced at them, shared by the lookups below
    fn priced(&self, model: &str, tokens: &CacheTokenBuckets) -> Option<(Rates, Decimal)> {
        let rates = self.rates_for(model)?;
        Some((rates, tokens_cost(rates, tokens)))
    }
}

impl CacheCostService for RatesCost {
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
        _moment: DateTime<Utc>,
    ) -> Option<CacheCostScenario> {
        let (_, input_cost) = self.priced(request.model, request.tokens)?;
        Some(CacheCostScenario {
            tokens: *request.tokens,
            input_cost,
        })
    }

    fn price(&self, request: &PriceRequest<'_>, _moment: DateTime<Utc>) -> Option<Decimal> {
        let (rates, input_cost) = self.priced(request.model, request.tokens)?;
        let completion = rates
            .output
            .checked_mul(Decimal::from(request.completion_tokens))?;
        input_cost.checked_add(completion)
    }
}

#[derive(Default)]
pub(super) struct FakeMinimum {
    pub(super) tokens: i64,
}

impl MinimumCacheTokens for FakeMinimum {
    fn minimum_tokens(&self, _model: &str) -> i64 {
        self.tokens
    }
}

// The fixture deployments declare their own ceilings, so the cost map answers no limit.
#[derive(Default)]
pub(super) struct NoLimits;

impl PriceLookup for NoLimits {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
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

// The question the fixture appends after the cache breakpoint; the prefix body cuts it off.
const FULL_PROMPT_TEXT: &str = "What is 2 + 2?";

// Counts by shape: the full prompt, the one carrying the question, is the total and the cut prefix is the prefix.
pub(super) struct ScriptedTransport {
    pub(super) total: Option<i64>,
    pub(super) prefix: Option<i64>,
    sent: Mutex<Vec<String>>,
}

impl ScriptedTransport {
    pub(super) fn new(total: Option<i64>, prefix: Option<i64>) -> Self {
        Self {
            total,
            prefix,
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> usize {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

// the scripted transport answers the total for the full prompt and the prefix for the body cut at the breakpoint
fn counted_tokens(bytes: &[u8], total: Option<i64>, prefix: Option<i64>) -> Option<i64> {
    let body = from_utf8(bytes).ok()?;
    Some(if body.contains(FULL_PROMPT_TEXT) {
        total
    } else {
        prefix
    })
}

impl CountTokensTransport for ScriptedTransport {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.url.to_string());
        let counted = match &request.body {
            RequestBody::Json(bytes) => counted_tokens(bytes, self.total, self.prefix),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => None,
        };
        let response = counted.map_or_else(
            || UpstreamResponse {
                status: 500,
                headers: HeaderMap::new(),
                body: Vec::new(),
            },
            |tokens| UpstreamResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: format!(r#"{{"input_tokens":{tokens}}}"#).into_bytes(),
            },
        );
        ready(Ok(response))
    }
}

// The proxy-state gates a case refuses through; a host carries the ones that are true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    ForeignRouter,
    RoutingPlugins,
    RequestTransforms,
    CompactionPending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LimiterMode {
    Grant,
    Deny,
    Hang,
}

pub(super) struct FakeLimiter {
    pub(super) mode: LimiterMode,
    grants: Mutex<Vec<String>>,
}

impl FakeLimiter {
    pub(super) fn granting() -> Self {
        Self {
            mode: LimiterMode::Grant,
            grants: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn denying() -> Self {
        Self {
            mode: LimiterMode::Deny,
            grants: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn hanging() -> Self {
        Self {
            mode: LimiterMode::Hang,
            grants: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn grants(&self) -> Vec<String> {
        self.grants
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct GrantedSlot;

impl CapacitySlot for GrantedSlot {}

#[async_trait]
impl CapacityLimiter for FakeLimiter {
    async fn capacity(
        &self,
        _caller: &UserAPIKeyAuth,
        model: &str,
        _request_data: &JsonMap,
    ) -> Result<Box<dyn CapacitySlot>, PreCallHookError> {
        self.grants
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        match self.mode {
            LimiterMode::Grant => Ok(Box::new(GrantedSlot)),
            LimiterMode::Deny => Err(PreCallHookError::Admission(
                WindowedAdmissionError::WindowPairCount { keys: 0, values: 1 },
            )),
            LimiterMode::Hang => pending().await,
        }
    }
}

// The deployment shapes the LiteLLM fixtures build through Router(model_list=...).
pub(super) fn deployment(model_name: &str, params: &str, info: &str) -> DeploymentSpec {
    DeploymentSpec {
        model_name: model_name.to_owned(),
        litellm_params: object(params),
        model_info: object(info),
        extra: JsonObject::new(),
    }
}

// the per-token prices a fixture deployment declares, the same ones Rates prices its costs with
struct DeploymentPrices {
    input: &'static str,
    output: &'static str,
    read: &'static str,
    creation: &'static str,
}

const CHEAP_PRICES: DeploymentPrices = DeploymentPrices {
    input: "0.000001",
    output: "0.000004",
    read: "0.0000001",
    creation: "0.00000125",
};

const STRONG_PRICES: DeploymentPrices = DeploymentPrices {
    input: "0.000003",
    output: "0.000015",
    read: "0.0000003",
    creation: "0.00000375",
};

fn priced_deployment(
    group: &str,
    model: &str,
    id: &str,
    max_output: i64,
    prices: &DeploymentPrices,
) -> DeploymentSpec {
    let params = format!(
        r#"{{"model":"anthropic/{model}","api_key":"{PROVIDER_KEY}","input_cost_per_token":{},"output_cost_per_token":{},"cache_read_input_token_cost":{},"cache_creation_input_token_cost":{}}}"#,
        prices.input, prices.output, prices.read, prices.creation
    );
    let info =
        format!(r#"{{"id":"{id}","max_input_tokens":100000,"max_output_tokens":{max_output}}}"#);
    deployment(group, &params, &info)
}

pub(super) fn cheap_deployment(max_output: i64) -> DeploymentSpec {
    priced_deployment("cheap", CHEAP_MODEL, CHEAP_ID, max_output, &CHEAP_PRICES)
}

pub(super) fn strong_deployment(max_output: i64) -> DeploymentSpec {
    priced_deployment(
        "strong",
        STRONG_MODEL,
        STRONG_ID,
        max_output,
        &STRONG_PRICES,
    )
}

pub(super) fn cold_deployment() -> DeploymentSpec {
    let params = format!(r#"{{"model":"anthropic/{CHEAP_MODEL}","api_key":"{PROVIDER_KEY}"}}"#);
    let info =
        format!(r#"{{"id":"{COLD_ID}","max_input_tokens":100000,"max_output_tokens":30000}}"#);
    deployment("cold", &params, &info)
}

// A bare auto_router marker, the alias the router's own deployments_for_request answers for.
pub(super) fn marker_deployment(params: &str) -> DeploymentSpec {
    deployment(
        "smart",
        params,
        r#"{"id":"test-cache-marker","max_input_tokens":100000,"max_output_tokens":30000}"#,
    )
}

pub(super) fn marker_params() -> String {
    r#"{"model":"auto_router/complexity_router","complexity_router_config":{"tiers":{"SIMPLE":"cheap"}}}"#
        .to_owned()
}

pub(super) fn tier_limits(entries: &[(&str, &str, i64)]) -> FakeTierParams {
    let mut limits = BTreeMap::new();
    for (tier, model, limit) in entries {
        let params = JsonMap::from([("max_tokens".to_owned(), number(*limit))]);
        limits.insert(((*tier).to_owned(), (*model).to_owned()), params);
    }
    FakeTierParams { limits }
}

// _litellm_params_for_model: the tier entry's own params, which max_tokens_from_tier_model fills with the ceiling.
#[derive(Default)]
pub(super) struct FakeTierParams {
    limits: BTreeMap<(String, String), JsonMap>,
}

impl TierParams for FakeTierParams {
    fn params_for_model(&self, tier: &str, model: &str) -> JsonMap {
        self.limits
            .get(&(tier.to_owned(), model.to_owned()))
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Default)]
pub(super) struct Fixture {
    pub(super) settings: AnthropicSettings,
    pub(super) resolution: Resolution,
    pub(super) cache: MemoryCache,
    pub(super) cost: RatesCost,
    pub(super) minimum: FakeMinimum,
    pub(super) prices: NoLimits,
}

// the provider key is the only setting the fixtures set past the Anthropic defaults
fn fixture_settings() -> AnthropicSettings {
    AnthropicSettings {
        api_key: Some(SecretString::from(PROVIDER_KEY)),
        auth_token: None,
        api_base: DEFAULT_API_BASE.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}

impl Fixture {
    pub(super) fn new() -> Self {
        let settings = fixture_settings();
        Self {
            settings,
            resolution: Resolution::default(),
            cache: MemoryCache::default(),
            cost: RatesCost::default(),
            minimum: FakeMinimum::default(),
            prices: NoLimits,
        }
    }

    pub(super) fn context(&self) -> ResolutionContext<'_> {
        self.resolution.context()
    }

    pub(super) fn host<'frame>(
        &'frame self,
        context: &'frame ResolutionContext<'frame>,
    ) -> FakeHost<'frame> {
        FakeHost {
            settings: &self.settings,
            resolution: context,
            cache: &self.cache,
            cost: &self.cost,
            minimum: &self.minimum,
            prices: &self.prices,
            handler: fixture_handler(),
            tiers: ladder(&[("SIMPLE", "cheap"), ("COMPLEX", "strong")]),
            features: Vec::new(),
            deployments: fixture_deployments(),
            authorized: fixture_authorized(),
            missing: Vec::new(),
            unhealthy: Vec::new(),
            refusals: Vec::new(),
            limiter: Some(FakeLimiter::granting()),
            clock: Mutex::new(VecDeque::new()),
            authorization: Mutex::new(Vec::new()),
        }
    }
}

fn fixture_handler() -> AnthropicCountTokensHandler<ScriptedTransport> {
    AnthropicCountTokensHandler {
        transport: ScriptedTransport::new(Some(51_000), Some(50_000)),
        request_timeout: Duration::from_secs(5),
        beta_version: DEFAULT_BETA_VERSION.to_owned(),
    }
}

fn fixture_deployments() -> BTreeMap<String, Vec<DeploymentSpec>> {
    BTreeMap::from([
        ("cheap".to_owned(), vec![cheap_deployment(30_000)]),
        ("strong".to_owned(), vec![strong_deployment(30_000)]),
        ("cold".to_owned(), vec![cold_deployment()]),
        (
            "smart".to_owned(),
            vec![marker_deployment(&marker_params())],
        ),
    ])
}

fn fixture_authorized() -> Vec<String> {
    vec!["cheap".to_owned(), "strong".to_owned(), "cold".to_owned()]
}

pub(super) struct FakeHost<'frame> {
    pub(super) settings: &'frame AnthropicSettings,
    pub(super) resolution: &'frame ResolutionContext<'frame>,
    pub(super) cache: &'frame MemoryCache,
    pub(super) cost: &'frame RatesCost,
    pub(super) minimum: &'frame FakeMinimum,
    pub(super) prices: &'frame NoLimits,
    pub(super) handler: AnthropicCountTokensHandler<ScriptedTransport>,
    pub(super) tiers: TierSet,
    pub(super) features: Vec<Feature>,
    pub(super) deployments: BTreeMap<String, Vec<DeploymentSpec>>,
    pub(super) authorized: Vec<String>,
    pub(super) missing: Vec<String>,
    pub(super) unhealthy: Vec<String>,
    pub(super) refusals: Vec<Refusal>,
    pub(super) limiter: Option<FakeLimiter>,
    pub(super) clock: Mutex<VecDeque<f64>>,
    authorization: Mutex<Vec<String>>,
}

impl FakeHost<'_> {
    fn deployments_named(&self, model: &str) -> Vec<DeploymentSpec> {
        self.deployments.get(model).cloned().unwrap_or_default()
    }

    pub(super) fn authorization_calls(&self) -> Vec<String> {
        self.authorization
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn requests(&self) -> usize {
        self.handler.transport.requests()
    }
}

#[async_trait]
impl CacheRoutingHost for FakeHost<'_> {
    type Transport = ScriptedTransport;

    fn routing_config(&self) -> CacheRoutingConfig<'_> {
        CacheRoutingConfig {
            tiers: &self.tiers,
            features: &self.features,
        }
    }

    fn prediction_services(&self) -> PredictionServices<'_> {
        PredictionServices {
            settings: self.settings,
            resolution: self.resolution,
            cache: self.cache,
            cost: self.cost,
            minimum: self.minimum,
            billing_time: billing_time(),
        }
    }

    fn price_lookup(&self) -> &dyn PriceLookup {
        self.prices
    }

    fn count_tokens_handler(&self) -> &AnthropicCountTokensHandler<Self::Transport> {
        &self.handler
    }

    fn capacity_limiter(&self) -> Option<&dyn CapacityLimiter> {
        let limiter: &dyn CapacityLimiter = self.limiter.as_ref()?;
        Some(limiter)
    }

    fn clock_seconds(&self) -> f64 {
        let mut readings = self.clock.lock().unwrap_or_else(PoisonError::into_inner);
        readings.pop_front().unwrap_or(NOW)
    }

    fn proxy_uses_this_router(&self) -> bool {
        !self.refusals.contains(&Refusal::ForeignRouter)
    }

    fn has_routing_plugins(&self) -> bool {
        self.refusals.contains(&Refusal::RoutingPlugins)
    }

    fn has_request_transforms(&self) -> bool {
        self.refusals.contains(&Refusal::RequestTransforms)
    }

    fn compaction_pending(&self, _request_kwargs: &JsonMap) -> bool {
        self.refusals.contains(&Refusal::CompactionPending)
    }

    fn deployments_for_request(
        &self,
        model: &str,
        _request_kwargs: &JsonMap,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
        if self.missing.iter().any(|name| name == model) {
            return Err(CatalogLookupError::NoHealthyDeployments {
                model: model.to_owned(),
            });
        }
        Ok(self.deployments_named(model))
    }

    async fn healthy_deployments(
        &self,
        model: &str,
        _messages: Option<&[Message]>,
        _request_kwargs: &JsonMap,
    ) -> Result<Vec<DeploymentSpec>, RoutingError> {
        let model_is_unhealthy = self.unhealthy.iter().any(|name| name == model);
        if model_is_unhealthy {
            return Err(RoutingError::NotBuilt {
                entry: "healthy_deployments",
            });
        }
        Ok(self.deployments_named(model))
    }

    async fn can_key_call_model(
        &self,
        _caller: &UserAPIKeyAuth,
        model: &str,
    ) -> Result<(), EntityModelAccessError> {
        self.authorization
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        if self.authorized.iter().any(|name| name == model) {
            Ok(())
        } else {
            Err(EntityModelAccessError::RecurseDepth {
                model: model.to_owned(),
            })
        }
    }
}

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::complexity_placement as section;
use betterlite_config::sections::context_placement as context_section;
use betterlite_config::sections::tier_definitions::TierParam;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::chat::ChatMessage;
use indexmap::IndexMap;
use rand::TryRng;
use serde_json::Number;

use crate::affinity_claims::AffinityClaims;
use crate::affinity_claims::local::LocalPins;
use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::context_placement::ContextPlacement;
use crate::context_placement::error::PlacementError;
use crate::context_placement::facts::{ModelLimitKey, RouterModelInfo};
use crate::context_placement::request::RequestCounter;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::AliasMap;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::router_predicates::JsonMap;
use crate::router_predicates::params::{ParamSupport, ResolvedProvider};
use crate::routing_groups::index::RoutingGroupIndex;
use crate::routing_value_store::StoreSettings;
use crate::tier_definitions::model::{TierModelEntry, TierPool, TierSet};

use super::super::TierPlacement;

pub(super) const SIMPLE: Tier = Tier::BuiltIn(ComplexityTier::Simple);
pub(super) const MEDIUM: Tier = Tier::BuiltIn(ComplexityTier::Medium);

// a 53-bit draw: 0, 2^51, 2^52, 3 * 2^51 and 2^53 - 1, the last sample below one
pub(super) const ZERO: u64 = 0;
pub(super) const QUARTER: u64 = 2_251_799_813_685_248;
pub(super) const HALF: u64 = 4_503_599_627_370_496;
pub(super) const THREE_QUARTERS: u64 = 6_755_399_441_055_744;
pub(super) const LAST_SAMPLE: u64 = 9_007_199_254_740_991;

pub(super) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

// a 53-bit draw sequence, as the oracle's injected samples give it
pub(super) struct ScriptedRng {
    samples: Vec<u64>,
    pub draws: usize,
}

impl ScriptedRng {
    pub(super) fn new(samples: &[u64]) -> Self {
        Self {
            samples: samples.to_vec(),
            draws: 0,
        }
    }

    fn sample(&mut self) -> u64 {
        let sample = self.samples.get(self.draws).copied().unwrap_or(0);
        self.draws = self.draws.saturating_add(1);
        sample
    }
}

impl TryRng for ScriptedRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        Ok(u32::try_from(self.sample().wrapping_shr(21)).unwrap_or(u32::MAX))
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        // the oracle's injected draw: the 53-bit sample in the high bits, so rand's f64 is sample/2^53
        Ok(self.sample().wrapping_shl(11) | 0x07FF)
    }

    fn try_fill_bytes(&mut self, _dst: &mut [u8]) -> Result<(), Infallible> {
        Ok(())
    }
}

// a tier whose value is one name must not reach the generator at all
pub(super) struct NoDraw;

impl TryRng for NoDraw {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        panic!("the placement drew from the generator")
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        panic!("the placement drew from the generator")
    }

    fn try_fill_bytes(&mut self, _dst: &mut [u8]) -> Result<(), Infallible> {
        panic!("the placement drew from the generator")
    }
}

pub(super) struct Limits;

impl RouterModelInfo for Limits {
    fn limit(&self, _group: &str, deployment: &DeploymentSpec, key: ModelLimitKey) -> Option<u64> {
        match deployment.model_info.get(key.field()) {
            Some(Json::Number(number)) => number.as_u64(),
            Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
            | None => None,
        }
    }
}

pub(super) struct Counter;

#[async_trait]
impl RequestCounter for Counter {
    async fn count(&self, _messages: &[ChatMessage], _text: &str) -> Result<u64, PlacementError> {
        Ok(0)
    }
}

pub(super) struct Support;

impl ParamSupport for Support {
    fn is_authenticating_provider(&self, _model: &str, _custom_llm_provider: Option<&str>) -> bool {
        false
    }

    fn resolve_provider(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<ResolvedProvider> {
        match model.split_once('/') {
            Some((provider, bare)) => Some(ResolvedProvider {
                model: bare.to_owned(),
                custom_llm_provider: provider.to_owned(),
            }),
            None => Some(ResolvedProvider {
                model: model.to_owned(),
                custom_llm_provider: "openai".to_owned(),
            }),
        }
    }

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: &str,
        _base_model: Option<&str>,
    ) -> Option<Vec<String>> {
        Some(vec!["max_tokens".to_owned(), "temperature".to_owned()])
    }
}

fn no_provider(_name: &str) -> Option<String> {
    None
}

pub(super) struct World {
    pub config: section::Config,
    pub tiers: TierSet,
    pub default_model: Option<String>,
    pub catalog: DeploymentCatalog,
    pub context_config: context_section::Config,
    pub model_info: Limits,
    pub counter: Counter,
    pub claims: AffinityClaims,
    pub model_name: String,
    pub affinity_config: String,
    pub deployment_pin: bool,
    pub ttl: u32,
    pub aliases: AliasMap,
    pub groups: RoutingGroupIndex,
    pub provider: fn(&str) -> Option<String>,
    pub support: Support,
}

impl World {
    pub(super) fn new() -> Self {
        let local = LocalPins::new(
            StoreSettings {
                max_size_in_memory: None,
                default_ttl_seconds: None,
                max_size_per_item: Some(1024),
                loaded_max_size_per_item_kib: 1024,
            },
            Arc::new(|| 0.0_f64),
        )
        .unwrap();
        Self {
            config: section::Config::default(),
            tiers: TierSet {
                definitions: None,
                pools: BTreeMap::new(),
                labels: BTreeMap::new(),
                non_reasoning: false,
                fallback: None,
            },
            default_model: None,
            catalog: DeploymentCatalog::default(),
            context_config: context_section::Config {
                enabled: false,
                buffer: 0.95,
            },
            model_info: Limits,
            counter: Counter,
            claims: AffinityClaims {
                local,
                shared: None,
            },
            model_name: "router".to_owned(),
            affinity_config: "affinity".to_owned(),
            deployment_pin: false,
            ttl: 60,
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
            provider: no_provider,
            support: Support,
        }
    }

    pub(super) fn add(&mut self, group: &str, model: &str, ceiling: Option<u64>) {
        self.catalog.push(deployment(group, model, ceiling, &[]));
    }

    pub(super) fn set_pool(&mut self, tier: &str, pool: TierPool) {
        self.tiers.pools.insert(tier.to_owned(), pool);
    }

    pub(super) fn context(&self) -> ContextPlacement<'_> {
        ContextPlacement {
            config: &self.context_config,
            deployments: &self.catalog,
            model_info: &self.model_info,
            counter: &self.counter,
        }
    }

    pub(super) fn service<'world>(
        &'world self,
        context: &'world ContextPlacement<'world>,
    ) -> TierPlacement<'world> {
        TierPlacement {
            config: &self.config,
            tiers: &self.tiers,
            default_model: self.default_model.as_deref(),
            context,
            claims: &self.claims,
            model_name: &self.model_name,
            tier_affinity_config: &self.affinity_config,
            deployment_pin: self.deployment_pin,
            session_affinity_ttl_seconds: self.ttl,
        }
    }

    pub(super) fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &self.provider,
        }
    }
}

pub(super) fn single(model: &str) -> TierPool {
    TierPool::Single(entry(model, &[]))
}

pub(super) fn pool(models: &[&str]) -> TierPool {
    TierPool::Pool(models.iter().map(|model| entry(model, &[])).collect())
}

pub(super) fn entry(model: &str, params: &[(&str, Json)]) -> TierModelEntry {
    let params: IndexMap<String, TierParam> = params
        .iter()
        .map(|(key, value)| ((*key).to_owned(), TierParam::Value(value.clone())))
        .collect();
    TierModelEntry {
        model_name: model.to_owned(),
        params,
    }
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(super) fn deployment(
    group: &str,
    model: &str,
    ceiling: Option<u64>,
    allowed: &[&str],
) -> DeploymentSpec {
    let mut litellm_params = JsonObject::new();
    litellm_params.insert("model".to_owned(), text(model));
    if !allowed.is_empty() {
        litellm_params.insert(
            "allowed_openai_params".to_owned(),
            Json::List(allowed.iter().map(|param| text(param)).collect()),
        );
    }
    let mut model_info = JsonObject::new();
    if let Some(ceiling) = ceiling {
        model_info.insert(
            "max_output_tokens".to_owned(),
            Json::Number(Number::from(ceiling)),
        );
    }
    DeploymentSpec {
        model_name: group.to_owned(),
        litellm_params,
        model_info,
        extra: JsonObject::new(),
    }
}

pub(super) fn request(fields: &[(&str, Json)]) -> JsonMap {
    fields
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

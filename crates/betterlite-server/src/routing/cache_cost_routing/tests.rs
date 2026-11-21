#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::cache_cost_routing as section;
use betterlite_config::sections::complexity_placement as placement_section;
use betterlite_config::sections::context_placement as context_section;
use betterlite_config::sections::tier_definitions::TierParam;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_router::affinity_claims::AffinityClaims;
use betterlite_router::affinity_claims::local::LocalPins;
use betterlite_router::cache_cost_routing::{CacheAwareChoice, CachedModelChooser, ChooseCached};
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::complexity_placement::TierPlacement;
use betterlite_router::complexity_scoring::tier::ComplexityTier;
use betterlite_router::context_placement::ContextPlacement;
use betterlite_router::context_placement::error::PlacementError;
use betterlite_router::context_placement::facts::{ModelLimitKey, RouterModelInfo};
use betterlite_router::context_placement::request::RequestCounter;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::router_predicates::JsonMap;
use betterlite_router::routing_value_store::StoreSettings;
use betterlite_router::tier_definitions::model::{TierModelEntry, TierPool, TierSet};
use serde_json::Number;

use super::{CacheCostRoutingService, PlacedTierParams};

// a tier entry with one ceiling key, so the lookup never needs the deployment catalog
struct World {
    placement_config: placement_section::Config,
    context_config: context_section::Config,
    tiers: TierSet,
    catalog: DeploymentCatalog,
    model_info: Limits,
    counter: Counter,
    claims: AffinityClaims,
}

impl World {
    fn new() -> Self {
        let mut pools = BTreeMap::new();
        pools.insert("MEDIUM".to_owned(), TierPool::Single(entry("medium", 4096)));
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
            placement_config: placement_section::Config::default(),
            context_config: context_section::Config {
                enabled: false,
                buffer: 0.95,
            },
            tiers: TierSet {
                definitions: None,
                pools,
                labels: BTreeMap::new(),
                non_reasoning: false,
                fallback: None,
            },
            catalog: DeploymentCatalog::default(),
            model_info: Limits,
            counter: Counter,
            claims: AffinityClaims {
                local,
                shared: None,
            },
        }
    }

    fn context(&self) -> ContextPlacement<'_> {
        ContextPlacement {
            config: &self.context_config,
            deployments: &self.catalog,
            model_info: &self.model_info,
            counter: &self.counter,
        }
    }

    fn placement<'world>(
        &'world self,
        context: &'world ContextPlacement<'world>,
    ) -> TierPlacement<'world> {
        TierPlacement {
            config: &self.placement_config,
            tiers: &self.tiers,
            default_model: None,
            context,
            claims: &self.claims,
            model_name: "router",
            tier_affinity_config: "affinity",
            deployment_pin: false,
            session_affinity_ttl_seconds: 60,
        }
    }
}

fn entry(model: &str, ceiling: u64) -> TierModelEntry {
    TierModelEntry {
        model_name: model.to_owned(),
        params: [(
            "max_tokens".to_owned(),
            TierParam::Value(Json::Number(Number::from(ceiling))),
        )]
        .into_iter()
        .collect(),
    }
}

fn ceiling(ceiling: u64) -> JsonMap {
    [("max_tokens".to_owned(), Json::Number(Number::from(ceiling)))]
        .into_iter()
        .collect()
}

struct Limits;

impl RouterModelInfo for Limits {
    fn limit(
        &self,
        _group: &str,
        _deployment: &DeploymentSpec,
        _key: ModelLimitKey,
    ) -> Option<u64> {
        None
    }
}

struct Counter;

#[async_trait]
impl RequestCounter for Counter {
    async fn count(&self, _messages: &[ChatMessage], _text: &str) -> Result<u64, PlacementError> {
        Ok(0)
    }
}

struct NoChoice;

#[async_trait]
impl CachedModelChooser for NoChoice {
    async fn choose(&self, _request: ChooseCached<'_>) -> Option<CacheAwareChoice> {
        None
    }
}

#[test]
fn the_tier_lookup_reads_the_placed_tier_entry() {
    let world = World::new();
    let context = world.context();
    let placement = world.placement(&context);
    let params = PlacedTierParams {
        placement: &placement,
    };
    assert_eq!(params.params_for_model("MEDIUM", "medium"), ceiling(4096));
    assert_eq!(params.params_for_model("MEDIUM", "small"), JsonMap::new());
    assert_eq!(params.params_for_model("SIMPLE", "medium"), JsonMap::new());
}

#[test]
fn the_service_hands_the_router_its_config_labels_and_tier_lookup() {
    let world = World::new();
    let context = world.context();
    let placement = world.placement(&context);
    let config = section::Config {
        enabled: true,
        output_tokens: 1024,
        timeout_ms: 2000,
    };
    let labels = BTreeMap::from([(ComplexityTier::Medium, "Standard".to_owned())]);
    let service = CacheCostRoutingService::new(&config, &labels, &placement, &NoChoice);
    let routing = service.routing();
    assert_eq!(routing.config, &config);
    assert_eq!(routing.tier_labels, &labels);
    assert_eq!(
        routing
            .params_for_model
            .params_for_model("MEDIUM", "medium"),
        ceiling(4096)
    );
}

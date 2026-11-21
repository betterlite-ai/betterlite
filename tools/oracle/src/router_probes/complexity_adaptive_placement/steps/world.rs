use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::complexity_placement as tier_section;
use betterlite_config::sections::context_placement as context_section;
use betterlite_openai::chat::ChatMessage;
use betterlite_router::adaptive_classification::config::AdaptiveRouterPreferences;
use betterlite_router::affinity_claims::AffinityClaims;
use betterlite_router::affinity_claims::local::LocalPins;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::complexity_adaptive_placement::facts::{ModelDeclaration, ModelFacts};
use betterlite_router::complexity_placement::TierPlacement;
use betterlite_router::complexity_scoring::tier::{ComplexityTier, Tier};
use betterlite_router::context_placement::ContextPlacement;
use betterlite_router::context_placement::error::PlacementError;
use betterlite_router::context_placement::facts::{ModelLimitKey, RouterModelInfo};
use betterlite_router::context_placement::request::RequestCounter;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::router_predicates::JsonMap;
use betterlite_router::routing_value_store::StoreSettings;
use betterlite_router::tier_definitions::model::{TierModelEntry, TierPool, TierSet};

use super::super::wire::{DeploymentInput, PoolInput};
use super::{decimal, invalid};
use crate::router_probes::error::OperationError;

const MODEL_NAME: &str = "probe";
const AFFINITY_CONFIG: &str = "probe-affinity";
const SESSION_TTL: u32 = 3600;

pub(super) fn tier(name: &str) -> Tier {
    ComplexityTier::built_in(name).map_or_else(|| Tier::Named(name.to_owned()), Tier::BuiltIn)
}

// the case's pools as the tier set the router reads; its name order is the case's order
fn tier_set(pools: &[PoolInput]) -> TierSet {
    TierSet {
        definitions: None,
        pools: pools
            .iter()
            .map(|pool| (pool.tier.clone(), tier_pool(pool)))
            .collect(),
        labels: BTreeMap::new(),
        non_reasoning: false,
        fallback: None,
    }
}

fn tier_pool(pool: &PoolInput) -> TierPool {
    TierPool::Pool(
        pool.models
            .iter()
            .map(|model| TierModelEntry {
                model_name: model.clone(),
                params: JsonMap::new(),
            })
            .collect(),
    )
}

// the model_list's first deployment of each name declares its preferences and price
pub(super) fn facts(deployments: &[DeploymentInput]) -> Result<ModelFacts, OperationError> {
    let mut declarations = Vec::with_capacity(deployments.len());
    for deployment in deployments {
        declarations.push(ModelDeclaration {
            model_name: deployment.model_name.clone(),
            preferences: deployment
                .quality_tier
                .map(|quality_tier| AdaptiveRouterPreferences {
                    quality_tier,
                    strengths: deployment.strengths.clone(),
                }),
            params_cost: deployment
                .params_cost
                .as_deref()
                .map(|text| decimal(text, "params_cost"))
                .transpose()?,
            info_cost: deployment
                .info_cost
                .as_deref()
                .map(|text| decimal(text, "info_cost"))
                .transpose()?,
        });
    }
    Ok(ModelFacts::from_declarations(&declarations))
}

// the router never reads the context gate on this path, but TierPlacement carries it
struct ProbeLimits;

impl RouterModelInfo for ProbeLimits {
    fn limit(
        &self,
        _group: &str,
        _deployment: &DeploymentSpec,
        _key: ModelLimitKey,
    ) -> Option<u64> {
        None
    }
}

struct NoCounter;

#[async_trait]
impl RequestCounter for NoCounter {
    async fn count(&self, _messages: &[ChatMessage], _text: &str) -> Result<u64, PlacementError> {
        Ok(0)
    }
}

pub(super) struct World {
    pub(super) tiers: TierSet,
    config: tier_section::Config,
    context_config: context_section::Config,
    deployments: DeploymentCatalog,
    claims: AffinityClaims,
    limits: ProbeLimits,
    counter: NoCounter,
    default_model: Option<String>,
}

impl World {
    pub(super) fn new(
        pools: &[PoolInput],
        default_model: Option<&str>,
    ) -> Result<Self, OperationError> {
        let local = LocalPins::new(
            StoreSettings {
                max_size_in_memory: None,
                default_ttl_seconds: None,
                max_size_per_item: Some(1024_i64),
                loaded_max_size_per_item_kib: 1024_i64,
            },
            Arc::new(|| 0.0_f64),
        )
        .map_err(invalid)?;
        Ok(Self {
            tiers: tier_set(pools),
            config: tier_section::Config {
                return_raw_model_name: false,
                max_tokens_from_tier_model: true,
            },
            context_config: context_section::Config {
                enabled: false,
                buffer: context_section::DEFAULT_BUFFER,
            },
            deployments: DeploymentCatalog::default(),
            claims: AffinityClaims {
                local,
                shared: None,
            },
            limits: ProbeLimits,
            counter: NoCounter,
            default_model: default_model.map(str::to_owned),
        })
    }

    pub(super) fn context(&self) -> ContextPlacement<'_> {
        ContextPlacement {
            config: &self.context_config,
            deployments: &self.deployments,
            model_info: &self.limits,
            counter: &self.counter,
        }
    }

    pub(super) fn placement<'deps>(
        &'deps self,
        context: &'deps ContextPlacement<'deps>,
    ) -> TierPlacement<'deps> {
        TierPlacement {
            config: &self.config,
            tiers: &self.tiers,
            default_model: self.default_model.as_deref(),
            context,
            claims: &self.claims,
            model_name: MODEL_NAME,
            tier_affinity_config: AFFINITY_CONFIG,
            deployment_pin: false,
            session_affinity_ttl_seconds: SESSION_TTL,
        }
    }
}

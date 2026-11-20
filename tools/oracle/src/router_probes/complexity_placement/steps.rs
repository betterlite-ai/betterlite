use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::complexity_placement as section;
use betterlite_config::sections::context_placement as context_section;
use betterlite_config::sections::tier_definitions::{TierEntry, TierPool as WireTierPool};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::chat::ChatMessage;
use betterlite_router::affinity_claims::AffinityClaims;
use betterlite_router::affinity_claims::local::LocalPins;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::complexity_placement::params_on_request::{
    TierTarget, pin_tier_params_onto_request, restore_client_ceiling_no_tier_pins,
};
use betterlite_router::complexity_placement::{TierChoice, TierPlacement};
use betterlite_router::complexity_scoring::tier::{ComplexityTier, Tier};
use betterlite_router::context_placement::ContextPlacement;
use betterlite_router::context_placement::error::PlacementError;
use betterlite_router::context_placement::facts::{ModelLimitKey, RouterModelInfo};
use betterlite_router::context_placement::request::RequestCounter;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::router_predicates::params::{ParamSupport, ResolvedProvider};
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_router::routing_value_store::StoreSettings;
use betterlite_router::tier_definitions::model::{TierModelEntry, TierPool, TierSet};
use indexmap::IndexMap;
use serde_json::Number;

use super::wire::{DeploymentInput, Input, Output, StepInput, StepOutput};
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

const MODEL_NAME: &str = "probe";
const AFFINITY_CONFIG: &str = "probe-affinity";
const SESSION_TTL: u32 = 3600;
const EXHAUSTED: &str = "the request has no sample left for this draw";

// the world's get_router_model_info: a deployment declares its output ceiling in model_info
struct ProbeLimits;

impl RouterModelInfo for ProbeLimits {
    fn limit(&self, _group: &str, deployment: &DeploymentSpec, key: ModelLimitKey) -> Option<u64> {
        match deployment.model_info.get(key.field()) {
            Some(Json::Number(number)) => number.as_u64(),
            Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
            | None => None,
        }
    }
}

// the placement never counts tokens, but ContextPlacement carries the collaborator
struct NoCounter;

#[async_trait]
impl RequestCounter for NoCounter {
    async fn count(&self, _messages: &[ChatMessage], _text: &str) -> Result<u64, PlacementError> {
        Ok(0)
    }
}

pub(super) struct Probe<'input> {
    input: &'input Input,
    config: section::Config,
    tiers: TierSet,
    deployments: DeploymentCatalog,
    claims: AffinityClaims,
    aliases: AliasMap,
    routing_groups: RoutingGroupIndex,
}

// the provider capability answer get_supported_openai_params gives, keyed by the resolved model id
impl ParamSupport for Probe<'_> {
    fn is_authenticating_provider(&self, _model: &str, _custom_llm_provider: Option<&str>) -> bool {
        false
    }

    fn resolve_provider(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ResolvedProvider> {
        match custom_llm_provider {
            Some(provider) => Some(ResolvedProvider {
                model: model.to_owned(),
                custom_llm_provider: provider.to_owned(),
            }),
            None => {
                let (provider, bare) = model.split_once('/')?;
                Some(ResolvedProvider {
                    model: bare.to_owned(),
                    custom_llm_provider: provider.to_owned(),
                })
            }
        }
    }

    fn supported_openai_params(
        &self,
        model: &str,
        _custom_llm_provider: &str,
        _base_model: Option<&str>,
    ) -> Option<Vec<String>> {
        self.input.supported_params.get(model).cloned()
    }
}

impl Probe<'_> {
    // one request per step over one pin store, as the hook sees a session's turns
    pub(super) async fn output(
        &self,
        random: &mut InjectedRandom<'_>,
    ) -> Result<Output, OperationError> {
        let context_config = context_section::Config {
            enabled: false,
            buffer: context_section::DEFAULT_BUFFER,
        };
        let model_info = ProbeLimits;
        let counter = NoCounter;
        let context = ContextPlacement {
            config: &context_config,
            deployments: &self.deployments,
            model_info: &model_info,
            counter: &counter,
        };
        let service = TierPlacement {
            config: &self.config,
            tiers: &self.tiers,
            default_model: self.input.default_model.as_deref(),
            context: &context,
            claims: &self.claims,
            model_name: MODEL_NAME,
            tier_affinity_config: AFFINITY_CONFIG,
            deployment_pin: self.input.deployment_pin,
            session_affinity_ttl_seconds: SESSION_TTL,
        };
        self.outputs(&service, random).await
    }

    async fn outputs(
        &self,
        service: &TierPlacement<'_>,
        random: &mut InjectedRandom<'_>,
    ) -> Result<Output, OperationError> {
        let lookup = CatalogLookup {
            catalog: &self.deployments,
            model_group_alias: &self.aliases,
            routing_groups: &self.routing_groups,
            inferred_provider: &no_provider,
        };
        let target = TierTarget {
            lookup: &lookup,
            support: self,
        };
        let mut steps = Vec::with_capacity(self.input.steps.len());
        for step in &self.input.steps {
            steps.push(step_output(service, &target, step, random).await?);
        }
        if random.exhausted {
            return Err(OperationError::SamplesExhausted {
                message: EXHAUSTED.to_owned(),
            });
        }
        Ok(Output {
            tier_pools: service.tier_pools(),
            steps,
        })
    }
}

fn invalid(error: impl Display) -> OperationError {
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}

// the hook's own order: the raw-name marker, the tier's model and pin, then the tier's params
async fn step_output(
    service: &TierPlacement<'_>,
    target: &TierTarget<'_, Probe<'_>>,
    step: &StepInput,
    random: &mut InjectedRandom<'_>,
) -> Result<StepOutput, OperationError> {
    let mut kwargs = step.request_kwargs.clone();
    service.mark_raw_model_name(&mut kwargs);
    let tier = tier(&step.tier);
    let choice = TierChoice {
        tier: &tier,
        allowed_models: step.allowed_models.as_deref(),
        retained_pin: None,
    };
    let placed = service
        .place_tier(&choice, &mut kwargs, random)
        .await
        .map_err(invalid)?;
    let pinned = pin_tier_params_onto_request(
        target,
        &placed.model,
        &placed.params,
        &mut kwargs,
        step.responses_call,
    )
    .map_err(invalid)?;
    if !pinned {
        restore_client_ceiling_no_tier_pins(&mut kwargs);
    }
    Ok(StepOutput {
        model: placed.model,
        tier: placed.tier,
        pinned,
        request_kwargs: kwargs,
    })
}

fn tier(name: &str) -> Tier {
    match ComplexityTier::built_in(name) {
        Some(built_in) => Tier::BuiltIn(built_in),
        None => Tier::Named(name.to_owned()),
    }
}

fn tier_set(tiers: &IndexMap<String, WireTierPool>) -> TierSet {
    TierSet {
        definitions: None,
        pools: tiers
            .iter()
            .map(|(name, pool)| (name.clone(), tier_pool(pool)))
            .collect(),
        labels: BTreeMap::new(),
        non_reasoning: false,
        fallback: None,
    }
}

fn tier_pool(pool: &WireTierPool) -> TierPool {
    match pool {
        WireTierPool::Single(entry) => TierPool::Single(tier_entry(entry)),
        WireTierPool::Pool(entries) => TierPool::Pool(entries.iter().map(tier_entry).collect()),
    }
}

fn tier_entry(entry: &TierEntry) -> TierModelEntry {
    match entry {
        TierEntry::Model(name) => TierModelEntry {
            model_name: name.clone(),
            params: IndexMap::new(),
        },
        TierEntry::Detailed(detailed) => TierModelEntry {
            model_name: detailed.model_name.clone(),
            params: detailed.litellm_params.clone(),
        },
    }
}

fn deployments(entries: &[DeploymentInput]) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for entry in entries {
        catalog.push(deployment_spec(entry));
    }
    catalog
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

// the deployment's litellm_params and model_info as the YAML declares them
fn deployment_spec(entry: &DeploymentInput) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert(
        "model".to_owned(),
        text(entry.model.as_deref().unwrap_or_default()),
    );
    if let Some(provider) = &entry.custom_llm_provider {
        params.insert("custom_llm_provider".to_owned(), text(provider));
    }
    let allowed: Vec<Json> = entry
        .allowed_openai_params
        .iter()
        .map(|param| text(param.as_str()))
        .collect();
    if !allowed.is_empty() {
        params.insert("allowed_openai_params".to_owned(), Json::List(allowed));
    }
    let mut model_info = JsonObject::new();
    if let Some(ceiling) = entry.max_output_tokens {
        model_info.insert(
            "max_output_tokens".to_owned(),
            Json::Number(Number::from(ceiling)),
        );
    }
    DeploymentSpec {
        model_name: entry.group.clone(),
        litellm_params: params,
        model_info,
        extra: JsonObject::new(),
    }
}

const fn no_provider(_name: &str) -> Option<String> {
    None
}

fn claims() -> Result<AffinityClaims, OperationError> {
    let local = LocalPins::new(
        StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: Some(1024_i64),
            loaded_max_size_per_item_kib: 1024_i64,
        },
        Arc::new(|| 0.0_f64),
    )
    .map_err(|error| OperationError::InvalidRequest {
        message: format!("the probe pin store could not start: {error}"),
    })?;
    Ok(AffinityClaims {
        local,
        shared: None,
    })
}

pub(super) fn probe(input: &Input) -> Result<Probe<'_>, OperationError> {
    Ok(Probe {
        input,
        config: section::Config {
            return_raw_model_name: input
                .return_raw_model_name
                .unwrap_or(section::DEFAULT_RETURN_RAW_MODEL_NAME),
            max_tokens_from_tier_model: input
                .max_tokens_from_tier_model
                .unwrap_or(section::DEFAULT_MAX_TOKENS_FROM_TIER_MODEL),
        },
        tiers: tier_set(&input.tiers),
        deployments: deployments(&input.deployments),
        claims: claims()?,
        aliases: AliasMap::new(),
        routing_groups: RoutingGroupIndex::default(),
    })
}

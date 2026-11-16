use std::collections::{BTreeMap, BTreeSet};

use betterlite_auth::agent::get_agent_by_id;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::agent_registry::AgentRegistryApi as _;
use betterlite_auth::provider_resolution::resolve_rate_limit_provider;
use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution, SnapshotError};
use betterlite_error::rate_limit::{
    ProxyRateLimitError, ProxyRateLimitErrorInit, RateLimitCategory, RateLimitDimension,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel, resolve as resolve_model,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    agent_lookup: Option<AgentLookupInput>,
    #[serde(default)]
    provider_resolution: Option<ProviderResolutionInput>,
    #[serde(default)]
    rate_limit_error: Option<RateLimitErrorInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentLookupInput {
    config_agents: Vec<Json>,
    registered_agents: Vec<Json>,
    lookup_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum RouterKind {
    Absent,
    Empty,
    Deployments,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderResolutionInput {
    model: Option<String>,
    router: RouterKind,
    deployments: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RateLimitErrorInput {
    detail: Value,
    headers: Option<BTreeMap<String, Value>>,
    category: Option<RateLimitCategory>,
    rate_limit_type: Option<RateLimitDimension>,
    model: Option<String>,
    llm_provider: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match (
            &input.agent_lookup,
            &input.provider_resolution,
            &input.rate_limit_error,
        ) {
            (Some(lookup), None, None) => agent_lookup(lookup),
            (None, Some(resolution), None) => provider_resolution(resolution),
            (None, None, Some(error)) => rate_limit_error(error),
            _ => Err(Box::new(HelperError {
                class: String::from("InvalidInput"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: String::from(
                    "send exactly one of agent_lookup, provider_resolution or rate_limit_error",
                ),
            })),
        }
    }
}

fn agent_lookup(input: &AgentLookupInput) -> HelperResult {
    let registry = AgentRegistry::default();
    for record in &input.registered_agents {
        let text = serde_json::to_string(record).or_value_error()?;
        let agent: AgentResponse = serde_json::from_str(&text).or_value_error()?;
        registry.register_agent(agent);
    }
    registry
        .load_agents_from_config(Some(&input.config_agents))
        .or_value_error()?;
    match get_agent_by_id(&registry, &input.lookup_id) {
        None => Ok(json!({"found": null})),
        Some(agent) => Ok(json!({
            "found": {
                "agent_id": agent.agent_id,
                "litellm_params": agent.litellm_params,
            }
        })),
    }
}

struct NativeResolution {
    catalog: ModelCatalog,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    registry: JSONProviderRegistry,
    policies: ProviderPolicies,
    globals: GlobalCredentials,
}

impl Default for NativeResolution {
    fn default() -> Self {
        Self {
            catalog: ModelCatalog::default(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

impl ProviderResolution for NativeResolution {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        let request = ResolutionRequest {
            model,
            custom_llm_provider: None,
            api_base: None,
            api_key: None,
        };
        let context = ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: self.policies.clone(),
            globals: self.globals.clone(),
        };
        resolve_model(&request, &context)
    }
}

struct FixtureRouter {
    deployments: Vec<Json>,
}

impl DeploymentSnapshot for FixtureRouter {
    fn model_list(&self) -> Result<Option<&[Json]>, SnapshotError> {
        Ok(Some(self.deployments.as_slice()))
    }
}

fn provider_resolution(input: &ProviderResolutionInput) -> HelperResult {
    let native = NativeResolution::default();
    let empty = FixtureRouter {
        deployments: Vec::new(),
    };
    let filled = FixtureRouter {
        deployments: input.deployments.clone(),
    };
    let router: Option<&dyn DeploymentSnapshot> = match &input.router {
        RouterKind::Absent => None,
        RouterKind::Empty => Some(&empty),
        RouterKind::Deployments => Some(&filled),
    };
    let (model, provider) = resolve_rate_limit_provider(input.model.as_deref(), &native, router);
    Ok(json!({"model": model, "provider": provider}))
}

fn rate_limit_error(input: &RateLimitErrorInput) -> HelperResult {
    let error = ProxyRateLimitError::new(ProxyRateLimitErrorInit {
        detail: input.detail.clone(),
        headers: input.headers.clone(),
        category: input.category,
        rate_limit_type: input.rate_limit_type,
        model: input.model.clone(),
        llm_provider: input.llm_provider.clone(),
    });
    Ok(json!({
        "status_code": 429,
        "headers": error.headers,
        "message": error.message,
        "category": error.category,
        "rate_limit_type": error.rate_limit_type,
        "model": error.model,
        "llm_provider": error.llm_provider,
        "detail": error.detail,
    }))
}

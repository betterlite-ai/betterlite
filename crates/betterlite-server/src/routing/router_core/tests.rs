#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_catalog::error::CatalogLookupError;
use betterlite_router::deployment_catalog::lookup::CatalogResolver;
use betterlite_router::deployment_catalog::types::{Authority, CandidateView};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::router_core::error::RoutingError;
use betterlite_router::router_core::pipeline::{self, Resolved, StrategyRoute};
use betterlite_router::router_core::route::{RouteContext, RoutePolicy, RouteRequest};
use betterlite_router::router_predicates::params::{ParamSupport, ResolvedProvider};
use betterlite_router::static_eligibility::request::RequestFacts;
use betterlite_router::static_eligibility::tokens::TokenLimits;
use betterlite_router::strategy_dispatch::groups::{RoutingGroups, RoutingScope};
use betterlite_router::strategy_dispatch::service::StrategyDispatch;
use betterlite_router::strategy_dispatch::strategy::RoutingStrategy;
use betterlite_router::strategy_dispatch::types::StrategyOverride;
use rand::SeedableRng as _;
use rand::rngs::StdRng;

use super::{RouteInputs, route};
use crate::routing::routing_state::RoutingState;

const RULES: &str = "fallback_generalizations:\n  rules:\n    - pattern: \"^acme-\"\n      model_info:\n        litellm_provider: acme\n    - pattern: \"^acme-\"\n      model_info:\n        supports_vision: true\n";

struct OpenView;

impl CandidateView for OpenView {
    fn routing_group(&self, _: &str, _: Option<&str>) -> Option<Vec<DeploymentSpec>> {
        None
    }

    fn filter_access_groups(
        &self,
        _: &str,
        deployments: Vec<DeploymentSpec>,
        _: Option<&str>,
    ) -> Vec<DeploymentSpec> {
        deployments
    }

    fn filter_reserved(
        &self,
        deployments: Vec<DeploymentSpec>,
        _: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
        Ok(deployments)
    }

    fn default_fallback(&self) -> Option<String> {
        None
    }

    fn inferred_provider(&self, _: &str) -> Option<String> {
        None
    }
}

struct NoLimits;

impl TokenLimits for NoLimits {
    fn max_input_tokens(&self, _spec: &DeploymentSpec, _model: &str) -> Option<usize> {
        None
    }
}

struct NoParams;

impl ParamSupport for NoParams {
    fn is_authenticating_provider(&self, _model: &str, _custom_llm_provider: Option<&str>) -> bool {
        false
    }

    fn resolve_provider(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<ResolvedProvider> {
        None
    }

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: &str,
        _base_model: Option<&str>,
    ) -> Option<Vec<String>> {
        None
    }
}

fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
}

fn spec() -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert(String::from("model"), Json::Text(String::from("acme-1")));
    let mut info = JsonObject::new();
    info.insert(String::from("id"), Json::Text(String::from("dep-1")));
    DeploymentSpec::new(
        String::from("g"),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn catalog(deployment: &DeploymentSpec) -> DeploymentCatalog {
    let mut index = DeploymentCatalog::default();
    index.model_names.insert(deployment.model_name.clone());
    index.index_patterns(deployment).unwrap();
    index.push(deployment.clone());
    index
}

fn request(facts: &RequestFacts) -> RouteRequest<'_> {
    RouteRequest {
        model: "g",
        facts,
        authority: Authority::default(),
        environment: None,
        specific_deployment: false,
        affinity_pinned: false,
        excluded_deployment_ids: &[],
        retry_skipped_deployment_ids: &[],
    }
}

fn inputs<'route>(
    request: &'route RouteRequest<'route>,
    limits: &'route (dyn TokenLimits + Sync),
    params: &'route (dyn ParamSupport + Sync),
) -> RouteInputs<'route> {
    RouteInputs {
        request,
        limits,
        params,
        policy: RoutePolicy::default(),
        clients: None,
    }
}

fn strategy_route<'route>(
    dispatch: &'route mut StrategyDispatch,
    groups: &'route RoutingGroups,
    shadowing: &'route BTreeSet<String>,
) -> StrategyRoute<'route> {
    StrategyRoute {
        dispatch,
        override_strategy: StrategyOverride::Absent,
        scope: RoutingScope { groups, shadowing },
    }
}

fn prepare_route<'route>(
    state: &'route RoutingState,
    inputs: RouteInputs<'route>,
    resolver: &CatalogResolver<'_>,
    routing: &mut StrategyRoute<'route>,
) -> Result<Resolved, RoutingError> {
    let context = RouteContext {
        request: inputs.request,
        services: state,
        limits: inputs.limits,
        params: inputs.params,
        policy: inputs.policy,
        clients: inputs.clients,
    };
    pipeline::prepare(resolver, &context, routing)
}

async fn selected(config: &Config) -> DeploymentSpec {
    let state = RoutingState::new(config);
    let deployment = spec();
    let index = catalog(&deployment);
    let aliases = BTreeMap::new();
    let view = OpenView;
    let resolver = CatalogResolver {
        catalog: &index,
        model_group_alias: &aliases,
        model_alias_map: &aliases,
        view: &view,
    };
    let facts = RequestFacts::default();
    let request = request(&facts);
    let (limits, params) = (NoLimits, NoParams);
    let inputs = inputs(&request, &limits, &params);
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::SimpleShuffle, None).unwrap();
    let groups = RoutingGroups::default();
    let shadowing = BTreeSet::new();
    let mut routing = strategy_route(&mut dispatch, &groups, &shadowing);
    let prepared = prepare_route(&state, inputs, &resolver, &mut routing);
    let mut random = StdRng::seed_from_u64(7);
    route(
        &state,
        inputs,
        routing.dispatch,
        prepared.unwrap(),
        &mut random,
    )
    .await
    .unwrap()
    .deployment
}

#[tokio::test]
async fn a_configured_rule_fills_the_deployment_the_route_selects() {
    let deployment = selected(&config(RULES)).await;
    assert_eq!(
        deployment.model_info.get("litellm_provider"),
        Some(&Json::Text(String::from("acme")))
    );
    assert_eq!(
        deployment.model_info.get("supports_vision"),
        Some(&Json::Bool(true))
    );
}

#[tokio::test]
async fn a_config_without_rules_leaves_the_selected_deployment_alone() {
    let deployment = selected(&config("model_list: []\n")).await;
    assert!(deployment.model_info.get("litellm_provider").is_none());
    assert!(deployment.model_info.get("supports_vision").is_none());
}

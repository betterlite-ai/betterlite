#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_core::message_utilities::prompt_cache_checks::PromptCacheSettings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::cache_accounting::dialect::ModelFacts;
use betterlite_prompts::cache_defaults::CacheFacts;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_affinity::FilterRequest;
use betterlite_router::deployment_catalog::error::CatalogLookupError;
use betterlite_router::deployment_catalog::lookup::CatalogResolver;
use betterlite_router::deployment_catalog::types::{Authority, CandidateView};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::prompt_affinity::service::{InjectionDefaults, TokenGate};
use betterlite_router::router_core::error::RoutingError;
use betterlite_router::router_core::pipeline::{self, Resolved, StrategyRoute};
use betterlite_router::router_core::route::{RouteContext, RoutePolicy, RouteRequest};
use betterlite_router::router_core::services::RouteServices;
use betterlite_router::router_predicates::params::{ParamSupport, ResolvedProvider};
use betterlite_router::static_eligibility::request::RequestFacts;
use betterlite_router::static_eligibility::tokens::TokenLimits;
use betterlite_router::strategy_dispatch::groups::{RoutingGroups, RoutingScope};
use betterlite_router::strategy_dispatch::service::StrategyDispatch;
use betterlite_router::strategy_dispatch::strategy::RoutingStrategy;
use betterlite_router::strategy_dispatch::types::StrategyOverride;
use rand::SeedableRng as _;
use rand::rngs::StdRng;

use super::{RouteInputs, RouterServices, route};
use crate::routing::prompt_affinity::PromptAffinityInputs;
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

struct AllowAll;

#[async_trait]
impl TokenGate for AllowAll {
    async fn meets_minimum(&self, _model: &str, _messages: &[Json], _minimum: Option<u64>) -> bool {
        true
    }
}

struct NoFacts;

impl ModelFacts for NoFacts {
    fn cache_breakpoint_flag(&self, _model: &str) -> Option<bool> {
        None
    }

    fn provider(&self, _model: &str) -> Option<String> {
        None
    }
}

impl CacheFacts for NoFacts {
    fn supports_prompt_caching(&self, _model: &str, _provider: &str) -> bool {
        false
    }
}

// the prompt caching check is off in these configs, so it never reads these inputs
fn prompt_inputs() -> PromptAffinityInputs<LocalMemoryOps> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 0.0_f64);
    let memory = LocalMemoryOps::new(1024, clock).unwrap();
    PromptAffinityInputs {
        cache: Arc::new(DualCache::new(
            Arc::new(memory),
            None,
            DualCacheSettings {
                default_in_memory_ttl: None,
                default_redis_ttl: None,
                batch_cache_expiry: None,
                max_redis_batch_cache_size: 1000,
                redis_delete_chunk_size: 1000,
            },
        )),
        gate: Arc::new(AllowAll),
        facts: Arc::new(NoFacts),
        catalog: Arc::new(ModelCatalog::parse(b"{}", UnknownFields::Warn).unwrap()),
        settings: PromptCacheSettings::default(),
        defaults: InjectionDefaults::default(),
    }
}

fn router_services(checks: &[&str]) -> RouterServices {
    let joined = checks.join(", ");
    let source =
        format!("model_list: []\nrouter_settings:\n  optional_pre_call_checks: [{joined}]\n");
    RouterServices::from_config(&config(&source), prompt_inputs()).unwrap()
}

fn caller_kwargs() -> Object {
    serde_json::from_str(r#"{"metadata": {"user_api_key_hash": "key"}}"#).unwrap()
}

fn spec(model: &str) -> DeploymentSpec {
    spec_with_id(model, "dep-1")
}

fn spec_with_id(model: &str, id: &str) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert(String::from("model"), Json::Text(model.to_owned()));
    let mut info = JsonObject::new();
    info.insert(String::from("id"), Json::Text(id.to_owned()));
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
    services: &'route dyn RouteServices,
    inputs: RouteInputs<'route>,
    resolver: &CatalogResolver<'_>,
    routing: &mut StrategyRoute<'route>,
) -> Result<Resolved, RoutingError> {
    let context = RouteContext {
        request: inputs.request,
        services,
        limits: inputs.limits,
        params: inputs.params,
        policy: inputs.policy,
        clients: inputs.clients,
    };
    pipeline::prepare(resolver, &context, routing)
}

async fn selected(config: &Config, model: &str) -> DeploymentSpec {
    let state = RoutingState::new(config);
    selected_via(&state, model, None).await
}

// one route through the given services, with the request arguments the caller carries
async fn selected_via(
    services: &dyn RouteServices,
    model: &str,
    kwargs: Option<&Object>,
) -> DeploymentSpec {
    let facts = RequestFacts::default();
    let request = request(&facts);
    let (limits, params) = (NoLimits, NoParams);
    let mut inputs = inputs(&request, &limits, &params);
    inputs.policy.prompt.request_kwargs = kwargs;
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::SimpleShuffle, None).unwrap();
    // the catalog view is not Sync, so its borrow ends before the route's await
    let prepared = {
        let deployment = spec(model);
        let index = catalog(&deployment);
        let aliases = BTreeMap::new();
        let view = OpenView;
        let resolver = CatalogResolver {
            catalog: &index,
            model_group_alias: &aliases,
            model_alias_map: &aliases,
            view: &view,
        };
        let groups = RoutingGroups::default();
        let shadowing = BTreeSet::new();
        let mut routing = strategy_route(&mut dispatch, &groups, &shadowing);
        prepare_route(services, inputs, &resolver, &mut routing)
    };
    let mut random = StdRng::seed_from_u64(7);
    route(
        services,
        inputs,
        &mut dispatch,
        prepared.unwrap(),
        &mut random,
    )
    .await
    .unwrap()
    .deployment
}

#[tokio::test]
async fn a_configured_rule_fills_the_deployment_the_route_selects() {
    let deployment = selected(&config(RULES), "acme-1").await;
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
async fn a_config_without_rules_fills_from_the_shipped_cost_maps_rules() {
    let deployment = selected(&config("model_list: []\n"), "claude-sonnet-4").await;
    assert_eq!(
        deployment.model_info.get("litellm_provider"),
        Some(&Json::Text(String::from("anthropic")))
    );
    assert_eq!(
        deployment.model_info.get("supports_vision"),
        Some(&Json::Bool(true))
    );
}

#[tokio::test]
async fn a_deployment_no_shipped_rule_matches_is_left_alone() {
    let deployment = selected(&config("model_list: []\n"), "acme-1").await;
    assert!(deployment.model_info.get("litellm_provider").is_none());
    assert!(deployment.model_info.get("supports_vision").is_none());
}

// Router.add_optional_pre_call_checks: the services carry the check the config named, and none when it names none
#[test]
fn the_router_services_carry_the_configured_check() {
    assert!(
        router_services(&["deployment_affinity"])
            .deployment_affinity()
            .is_some()
    );
    assert!(router_services(&[]).deployment_affinity().is_none());
}

// async_pre_call_deployment_hook runs after the route selected, so the caller's next request follows the same deployment
#[tokio::test]
async fn the_route_records_the_selected_deployment_for_the_caller() {
    let services = router_services(&["deployment_affinity"]);
    let kwargs = caller_kwargs();
    let chosen = selected_via(&services, "claude-sonnet-4", Some(&kwargs)).await;
    assert_eq!(chosen.id(), Some("dep-1"));
    let pool = vec![chosen.clone(), spec_with_id("claude-sonnet-4", "dep-2")];
    let filter_kwargs = JsonObject::from([(
        String::from("metadata"),
        Json::Object(JsonObject::from([(
            String::from("user_api_key_hash"),
            Json::Text(String::from("key")),
        )])),
    )]);
    let selected = services
        .deployment_affinity()
        .unwrap()
        .filter_deployments(&FilterRequest {
            model: "g",
            deployments: &pool,
            request_kwargs: Some(&filter_kwargs),
            target_order: None,
        })
        .await
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id(), Some("dep-1"));
}

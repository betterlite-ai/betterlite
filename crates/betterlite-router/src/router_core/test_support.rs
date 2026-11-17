use crate::deployment_catalog::types::Authority;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::route::{RouteContext, RoutePolicy, RouteRequest};
use crate::router_core::services::RouteServices;
use crate::router_predicates::params::{ParamSupport, ResolvedProvider};
use crate::static_eligibility::request::RequestFacts;
use crate::static_eligibility::tokens::TokenLimits;
use crate::strategy::StrategyContext;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::types::Resolution;

// A router that keeps no dynamic state: every insertion point answers its default.
pub struct NoServices;

impl RouteServices for NoServices {}

// A model registry that declares no context window.
pub struct NoLimits;

impl TokenLimits for NoLimits {
    fn max_input_tokens(&self, _spec: &DeploymentSpec, _model: &str) -> Option<usize> {
        None
    }
}

// A provider registry that knows no provider.
pub struct NoParams;

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

// A request that names one model and asks for nothing else.
#[must_use]
pub fn route_request<'request>(
    model: &'request str,
    facts: &'request RequestFacts,
) -> RouteRequest<'request> {
    RouteRequest {
        model,
        facts,
        authority: Authority::default(),
        environment: None,
        specific_deployment: false,
        affinity_pinned: false,
        excluded_deployment_ids: &[],
        retry_skipped_deployment_ids: &[],
    }
}

// A route over the given request and stateless services.
#[must_use]
pub fn route_context<'route>(
    request: &'route RouteRequest<'route>,
    services: &'route dyn RouteServices,
    limits: &'route (dyn TokenLimits + Sync),
    params: &'route (dyn ParamSupport + Sync),
) -> RouteContext<'route> {
    RouteContext {
        request,
        services,
        limits,
        params,
        policy: RoutePolicy::default(),
        clients: None,
    }
}

// A strategy call over the given pool and selector state.
#[must_use]
pub fn strategy_context<'context>(
    deployments: &'context [DeploymentSpec],
    route: &'context RouteContext<'context>,
    dispatch: &'context mut StrategyDispatch,
    resolution: &'context Resolution,
) -> StrategyContext<'context> {
    StrategyContext {
        deployments,
        route,
        dispatch,
        resolution,
    }
}

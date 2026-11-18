use betterlite_router::router_core::attempt::AttemptResult;
use betterlite_router::router_core::error::RoutingError;
use betterlite_router::router_core::pipeline::{self, Resolved};
use betterlite_router::router_core::route::{
    ClientAccess, RouteContext, RoutePolicy, RouteRequest,
};
use betterlite_router::router_predicates::params::ParamSupport;
use betterlite_router::static_eligibility::tokens::TokenLimits;
use betterlite_router::strategy_dispatch::service::StrategyDispatch;
use rand::Rng;

use crate::routing::routing_state::RoutingState;

mod tests;

// Everything one server route reads besides the catalog's answer: the request, the typed services and the settings.
#[derive(Clone, Copy)]
pub struct RouteInputs<'route> {
    pub request: &'route RouteRequest<'route>,
    pub limits: &'route (dyn TokenLimits + Sync),
    pub params: &'route (dyn ParamSupport + Sync),
    pub policy: RoutePolicy,
    pub clients: Option<ClientAccess<'route>>,
}

// Router.async_get_available_deployment over one server route: every stage reads the config's rules through the routing state.
pub async fn route<R>(
    state: &RoutingState,
    inputs: RouteInputs<'_>,
    dispatch: &mut StrategyDispatch,
    resolved: Resolved,
    random: &mut R,
) -> Result<AttemptResult, RoutingError>
where
    R: Rng + Send,
{
    let context = RouteContext {
        request: inputs.request,
        services: state,
        limits: inputs.limits,
        params: inputs.params,
        policy: inputs.policy,
        clients: inputs.clients,
    };
    pipeline::route(&context, dispatch, resolved, random).await
}

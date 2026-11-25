use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::pass_through_registry::PassThroughRegistry;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::virtual_key_routes::caller::RouteCaller;
use betterlite_auth::virtual_key_routes::route_checks::{
    CalledRoute, is_virtual_key_allowed_to_call_route,
};
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_identity::AuthContext;

mod tests;

// what the async auth service resolved before this stage: the caller's key
#[derive(Debug, Default)]
pub struct VirtualKeyRouteRule {
    pub key: Option<UserAPIKeyAuth>,
}

#[derive(Clone, Debug)]
pub struct VirtualKeyRouteStageInput {
    pub context: AuthContext,
    pub rule: Arc<VirtualKeyRouteRule>,
    pub team_allowed_routes: Vec<String>,
    pub registry: PassThroughRegistry,
    pub route: String,
    pub method: Option<String>,
}

pub(crate) fn run_stage(
    stage: &VirtualKeyRouteStageInput,
) -> Result<AuthContext, Box<ProxyException>> {
    let caller = match stage.rule.key.as_ref() {
        Some(key) => RouteCaller::of_key(key),
        None => RouteCaller::absent(),
    };
    let called = CalledRoute {
        route: &stage.route,
        method: stage.method.as_deref(),
    };
    is_virtual_key_allowed_to_call_route(called, &caller, &stage.registry)?;
    Ok(stage.context.clone())
}

pub(crate) fn rule_for_request(request: &Request<Body>) -> Arc<VirtualKeyRouteRule> {
    request
        .extensions()
        .get::<Arc<VirtualKeyRouteRule>>()
        .map_or_else(|| Arc::new(VirtualKeyRouteRule::default()), Arc::clone)
}

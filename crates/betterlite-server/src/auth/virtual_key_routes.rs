use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::pass_through_registry::PassThroughRegistry;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::virtual_key_routes::caller::RouteCaller;
use betterlite_auth::virtual_key_routes::route_checks::{CalledRoute, check_virtual_key_route};
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::auth::route_match::decoded_route_path;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

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

pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let (team_allowed_routes, registry) = match request.extensions().get::<Arc<ServerState>>() {
        Some(state) => (
            state.virtual_key_routes.team_allowed_routes.clone(),
            state.virtual_key_routes.registry_snapshot(),
        ),
        None => (
            Vec::new(),
            PassThroughRegistry {
                root_path: String::new(),
                registrations: Vec::new(),
            },
        ),
    };
    let route =
        decoded_route_path(request.uri().path()).unwrap_or_else(|| request.uri().path().to_owned());
    let stage = VirtualKeyRouteStageInput {
        context,
        rule: rule_for_request(request),
        team_allowed_routes,
        registry,
        route,
        method: Some(request.method().as_str().to_uppercase()),
    };
    Ok(StageInput::AuthVirtualKeyRoutes(Box::new(stage)))
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
    check_virtual_key_route(called, &caller, &stage.registry)?;
    Ok(stage.context.clone())
}

pub(crate) fn rule_for_request(request: &Request<Body>) -> Arc<VirtualKeyRouteRule> {
    request
        .extensions()
        .get::<Arc<VirtualKeyRouteRule>>()
        .map_or_else(|| Arc::new(VirtualKeyRouteRule::default()), Arc::clone)
}

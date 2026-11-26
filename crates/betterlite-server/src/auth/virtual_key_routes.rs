use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::pass_through_registry::PassThroughRegistry;
use betterlite_auth::virtual_key_routes::caller::{AllowedRoutes, RouteCaller};
use betterlite_auth::virtual_key_routes::route_checks::{CalledRoute, check_virtual_key_route};
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::auth::route_match::decoded_route_path;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

#[derive(Clone, Debug)]
pub struct VirtualKeyRouteStageInput {
    pub context: AuthContext,
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
    let (team_allowed_routes, registry) =
        request.extensions().get::<Arc<ServerState>>().map_or_else(
            || {
                (
                    Vec::new(),
                    PassThroughRegistry {
                        root_path: String::new(),
                        registrations: Vec::new(),
                    },
                )
            },
            |server| {
                (
                    server.virtual_key_routes.team_allowed_routes.clone(),
                    server.virtual_key_routes.registry_snapshot(),
                )
            },
        );
    let route =
        decoded_route_path(request.uri().path()).unwrap_or_else(|| request.uri().path().to_owned());
    let stage = VirtualKeyRouteStageInput {
        context,
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
    // the auth chain resolves the caller's key into the principal, so the grants read it there
    let caller = stage.context.principal.source_key.0.as_ref().map_or_else(
        || RouteCaller {
            allowed_routes: AllowedRoutes::Unset,
            metadata: None,
            team_metadata: None,
            jwt_claims: false,
            has_token: false,
            team_id: None,
        },
        RouteCaller::of_key,
    );
    let requested = CalledRoute {
        route: &stage.route,
        method: stage.method.as_deref(),
    };
    check_virtual_key_route(requested, &caller, &stage.registry)?;
    Ok(stage.context.clone())
}

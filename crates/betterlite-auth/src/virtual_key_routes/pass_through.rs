use betterlite_error::proxy_exception::ProxyException;

use crate::pass_through_registry::PassThroughRegistry;

use super::caller::{
    RouteCaller, check_passthrough_route_access, jwt_team_routes_grant_pass_through,
};
use super::denial::auth_pass_through_denied;

#[must_use]
pub fn is_auth_enforced_pass_through_route(
    route: &str,
    method: Option<&str>,
    registry: &PassThroughRegistry,
) -> bool {
    registry
        .get_registered(route, method)
        .is_some_and(|metadata| metadata.auth == Some(true))
}

pub fn require_auth_pass_through_access(
    route: &str,
    caller: &RouteCaller<'_>,
    jwt_team_allowed_routes: &[String],
) -> Result<(), Box<ProxyException>> {
    if check_passthrough_route_access(route, caller) {
        return Ok(());
    }
    if jwt_team_routes_grant_pass_through(route, jwt_team_allowed_routes) {
        return Ok(());
    }
    Err(auth_pass_through_denied(route))
}

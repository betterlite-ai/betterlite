use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_routes::wildcard::route_matches_wildcard_pattern;

use crate::auxiliary_routes::AGENT_MANAGEMENT_ROUTES;
use crate::pass_through_registry::PassThroughRegistry;
use crate::route_match::classifier::is_mcp_discovery_route;
use crate::route_match::groups::get_actual_routes;
use crate::route_match::templates::{check_route_access, route_matches_allowed_route};

use super::caller::{
    AllowedRoutes, RouteCaller, check_passthrough_route_access, password_reset_required, text_entry,
};
use super::denial;
use super::groups::{AUTH_ENFORCED_GROUPS, group_routes};
use super::pass_through::is_protected_pass_through;

const AUTO_ROUTER_SESSION: &str = "/auto_router/session";
const LLM_API_ROUTES: &str = "llm_api_routes";

// the requested route, and the method already uppercased as LiteLLM's _get_request_method returns it
#[derive(Clone, Copy, Debug)]
pub struct CalledRoute<'route> {
    pub route: &'route str,
    pub method: Option<&'route str>,
}

#[derive(Clone, Copy, Debug, Default)]
struct GroupScan {
    granted: bool,
    pass_through_denied: bool,
}

#[derive(Clone, Copy, Debug)]
enum LlmApiOutcome {
    Granted,
    ProtectedWithoutAccess,
    NotMatched,
}

pub fn check_virtual_key_route(
    requested: CalledRoute<'_>,
    caller: &RouteCaller<'_>,
    registry: &PassThroughRegistry,
) -> Result<(), Box<ProxyException>> {
    let Some(routes) = restricted_routes(caller.allowed_routes) else {
        return Ok(());
    };
    let route = requested.route;
    if routes
        .iter()
        .filter_map(text_entry)
        .any(|allowed| route_matches_allowed_route(route, allowed))
    {
        return Ok(());
    }
    let scan = scan_groups(requested, routes, caller, registry);
    // an exact or wildcard entry returns before the remembered protected pass-through denial
    if scan.granted
        || routes
            .iter()
            .filter_map(text_entry)
            .any(|pattern| route_matches_wildcard_pattern(route, pattern))
    {
        return Ok(());
    }
    if scan.pass_through_denied {
        return Err(denial::auth_pass_through_denied(route));
    }
    if password_reset_required(caller) {
        return Err(denial::password_reset_denied());
    }
    let allowed = routes.iter().filter_map(text_entry).collect::<Vec<_>>();
    Err(denial::virtual_route_denied(route, &allowed))
}

const fn restricted_routes(allowed: AllowedRoutes<'_>) -> Option<&[JsonData]> {
    match allowed {
        AllowedRoutes::Unset | AllowedRoutes::NotAList | AllowedRoutes::Listed([]) => None,
        AllowedRoutes::Listed(routes) => Some(routes),
    }
}

fn scan_groups(
    requested: CalledRoute<'_>,
    routes: &[JsonData],
    caller: &RouteCaller<'_>,
    registry: &PassThroughRegistry,
) -> GroupScan {
    let mut scan = GroupScan {
        granted: false,
        pass_through_denied: false,
    };
    for name in routes.iter().filter_map(text_entry) {
        if get_actual_routes(&[name]) == [name.to_owned()] {
            continue;
        }
        if apply_group(requested, name, caller, registry, &mut scan) {
            return scan;
        }
    }
    scan
}

// true when the entry granted the route, so the caller stops scanning
fn apply_group(
    requested: CalledRoute<'_>,
    name: &str,
    caller: &RouteCaller<'_>,
    registry: &PassThroughRegistry,
    scan: &mut GroupScan,
) -> bool {
    let expanded = group_routes(name, registry);
    let allowed: Vec<&str> = expanded.iter().map(String::as_str).collect();
    if check_route_access(requested.route, &allowed) {
        let protected = AUTH_ENFORCED_GROUPS.contains(&name)
            && is_protected_pass_through(requested.route, requested.method, registry);
        if !protected {
            scan.granted = true;
            return true;
        }
        if check_passthrough_route_access(requested.route, caller) {
            scan.granted = true;
            return true;
        }
        scan.pass_through_denied = true;
    }
    if name == LLM_API_ROUTES {
        match llm_api_outcome(requested, caller, registry) {
            LlmApiOutcome::Granted => {
                scan.granted = true;
                return true;
            }
            LlmApiOutcome::ProtectedWithoutAccess => scan.pass_through_denied = true,
            LlmApiOutcome::NotMatched => {}
        }
    }
    false
}

// the llm_api_routes carve-outs: the session read, registered pass-throughs, MCP discovery reads and the agent registry
fn llm_api_outcome(
    requested: CalledRoute<'_>,
    caller: &RouteCaller<'_>,
    registry: &PassThroughRegistry,
) -> LlmApiOutcome {
    if requested.route == AUTO_ROUTER_SESSION && requested.method == Some("GET") {
        return LlmApiOutcome::Granted;
    }
    if registry.is_registered(requested.route) {
        if !is_protected_pass_through(requested.route, requested.method, registry) {
            return LlmApiOutcome::Granted;
        }
        return if check_passthrough_route_access(requested.route, caller) {
            LlmApiOutcome::Granted
        } else {
            LlmApiOutcome::ProtectedWithoutAccess
        };
    }
    if is_mcp_discovery_route(requested.route, requested.method) {
        return LlmApiOutcome::Granted;
    }
    if check_route_access(requested.route, AGENT_MANAGEMENT_ROUTES) {
        return LlmApiOutcome::Granted;
    }
    LlmApiOutcome::NotMatched
}

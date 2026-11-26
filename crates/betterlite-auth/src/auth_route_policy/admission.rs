use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;
use betterlite_routes::wildcard::route_matches_wildcard_pattern;

use crate::auth_identity::roles::{get_user_role, is_user_proxy_admin};
use crate::auxiliary_routes::{INFO_ROUTES, SELF_MANAGED_ROUTES};
use crate::management_routes::{
    GLOBAL_SPEND_TRACKING_ROUTES, INTERNAL_USER_VIEW_ONLY_ROUTES, TEAM_SERVICE_ACCOUNT_ROUTES,
    internal_user_routes, org_admin_allowed_routes,
};
use crate::organization_route_policy::membership::{OrganizationMembership, RouteUser};
use crate::organization_route_policy::org_admin::user_is_org_admin;
use crate::pass_through_registry::PassThroughRegistry;
use crate::resolved_key::auth::{UserAPIKeyAuth, is_team_service_account};
use crate::route_match::classifier::is_llm_api_route;
use crate::route_match::templates::{RouteValue, check_route_access, route_matches_allowed_route};
use crate::user_roles::UserRole;
use crate::virtual_key_routes::caller::{
    AllowedRoutes, RouteCaller, check_passthrough_route_access, jwt_team_allowed_routes, text_entry,
};
use crate::virtual_key_routes::pass_through::{
    is_protected_pass_through, require_pass_through_access,
};

use super::denial;
use super::mask::mask_user_id;
use super::viewer;

const USER_INFO_ROUTE: &str = "/user/info";
const SPEND_PERMISSION: &str = "get_spend_routes";
const MCP_API_PREFIX: &str = "/v1/mcp/";
const MCP_REST_PREFIX: &str = "/mcp-rest/";
const UNKNOWN: &str = "unknown";

// everything RouteChecks.non_proxy_admin_allowed_routes_check reads: the route, the caller's key and user row,
// and the configuration and registrations that grant access.
#[derive(Debug)]
pub struct RoutePolicyInput<'a> {
    pub route: &'a str,
    pub method: Option<&'a str>,
    pub user_id_query: Option<&'a str>,
    pub body: &'a JsonObject,
    pub token: Option<&'a UserAPIKeyAuth>,
    pub user: Option<&'a LiteLLMUserTable>,
    pub registry: &'a PassThroughRegistry,
    pub admin_only_routes: &'a [String],
    pub team_allowed_routes: &'a [String],
}

// auth_checks._is_api_route_allowed: the token must exist even for a proxy admin, and only a live user row
// with role proxy_admin skips the non-admin chain.
pub fn is_api_route_allowed(input: &RoutePolicyInput<'_>) -> Result<(), Box<ProxyException>> {
    let role = get_user_role(input.user);
    let Some(token) = input.token else {
        return Err(denial::invalid_token());
    };
    if is_user_proxy_admin(input.user) {
        return Ok(());
    }
    non_proxy_admin_allowed_routes_check(input, role, token)
}

fn non_proxy_admin_allowed_routes_check(
    input: &RoutePolicyInput<'_>,
    role: Option<UserRole>,
    token: &UserAPIKeyAuth,
) -> Result<(), Box<ProxyException>> {
    custom_admin_only_route_check(input)?;
    let caller = RouteCaller::of_key(token);
    if is_protected_pass_through(input.route, input.method, input.registry) {
        let granted = jwt_team_allowed_routes(&caller, input.team_allowed_routes);
        return require_pass_through_access(input.route, &caller, granted);
    }
    if is_llm_api_route(&RouteValue::Text(input.route)) {
        return Ok(());
    }
    if INFO_ROUTES.contains(&input.route) {
        return check_info_route(input, role, token);
    }
    role_branches(input, role, token, &caller)
}

// general_settings.admin_only_routes holds exact routes, and betterlite enforces the list without the license
fn custom_admin_only_route_check(input: &RoutePolicyInput<'_>) -> Result<(), Box<ProxyException>> {
    if input
        .admin_only_routes
        .iter()
        .any(|route| route == input.route)
    {
        return Err(denial::custom_admin_only(input.route));
    }
    Ok(())
}

fn check_info_route(
    input: &RoutePolicyInput<'_>,
    role: Option<UserRole>,
    token: &UserAPIKeyAuth,
) -> Result<(), Box<ProxyException>> {
    if input.route != USER_INFO_ROUTE {
        return Ok(());
    }
    let Some(requested) = input.user_id_query.filter(|user_id| !user_id.is_empty()) else {
        return Ok(());
    };
    let own = token.view.token.user_id.as_deref();
    if Some(requested) != own && role != Some(UserRole::ProxyAdminViewer) {
        return Err(denial::other_user_info(input.route, requested, own));
    }
    Ok(())
}

fn role_branches(
    input: &RoutePolicyInput<'_>,
    role: Option<UserRole>,
    token: &UserAPIKeyAuth,
    caller: &RouteCaller<'_>,
) -> Result<(), Box<ProxyException>> {
    if global_spend_allowed(input, token) {
        return Ok(());
    }
    if role == Some(UserRole::ProxyAdminViewer) {
        return viewer::check_access(input.route, input.method, input.body);
    }
    if broad_roles_allow(input, role) {
        return Ok(());
    }
    if input.route.starts_with(MCP_API_PREFIX) || input.route.starts_with(MCP_REST_PREFIX) {
        return Ok(());
    }
    if passthrough_or_service_account(input, token, caller) {
        return Ok(());
    }
    allowed_routes_fallback(input, caller)
}

fn global_spend_allowed(input: &RoutePolicyInput<'_>, token: &UserAPIKeyAuth) -> bool {
    GLOBAL_SPEND_TRACKING_ROUTES.contains(&input.route)
        && token
            .view
            .token
            .permissions
            .0
            .contains_key(SPEND_PERMISSION)
}

// internal users, organization admins, view-only internal users and self-managed routes, in that order
fn broad_roles_allow(input: &RoutePolicyInput<'_>, role: Option<UserRole>) -> bool {
    (role == Some(UserRole::InternalUser)
        && check_route_access(input.route, &internal_user_routes()))
        || (is_org_admin(input) && check_route_access(input.route, &org_admin_allowed_routes()))
        || (role == Some(UserRole::InternalUserViewer)
            && check_route_access(input.route, INTERNAL_USER_VIEW_ONLY_ROUTES))
        || check_route_access(input.route, SELF_MANAGED_ROUTES)
}

fn is_org_admin(input: &RoutePolicyInput<'_>) -> bool {
    let Some(user) = input.user else {
        return false;
    };
    let memberships = user.organization_memberships.as_ref().map(|rows| {
        rows.iter()
            .map(|row| OrganizationMembership {
                organization_id: Some(row.organization_id.clone()),
                user_role: row.user_role.clone(),
            })
            .collect()
    });
    let route_user = RouteUser {
        user_role: user.user_role.clone(),
        organization_memberships: memberships,
    };
    user_is_org_admin(input.body, Some(&route_user))
}

fn passthrough_or_service_account(
    input: &RoutePolicyInput<'_>,
    token: &UserAPIKeyAuth,
    caller: &RouteCaller<'_>,
) -> bool {
    check_passthrough_route_access(input.route, caller)
        || (is_team_service_account(token)
            && check_route_access(input.route, TEAM_SERVICE_ACCOUNT_ROUTES))
}

// the last resort: every entry is an exact route, a prefix or a trailing-wildcard pattern, and no entry at
// all is the same refusal as an unmatched list
fn allowed_routes_fallback(
    input: &RoutePolicyInput<'_>,
    caller: &RouteCaller<'_>,
) -> Result<(), Box<ProxyException>> {
    let AllowedRoutes::Listed(routes) = caller.allowed_routes else {
        return Err(admin_only_denial(input.user, input.route));
    };
    let allowed = routes.iter().filter_map(text_entry).any(|entry| {
        route_matches_allowed_route(input.route, entry)
            || route_matches_wildcard_pattern(input.route, entry)
    });
    if allowed {
        return Ok(());
    }
    Err(admin_only_denial(input.user, input.route))
}

fn admin_only_denial(user: Option<&LiteLLMUserTable>, route: &str) -> Box<ProxyException> {
    let role = user
        .and_then(|row| row.user_role.as_deref())
        .filter(|role| !role.is_empty())
        .unwrap_or(UNKNOWN);
    let stored_id = user.map_or(UNKNOWN, |row| row.user_id.as_str());
    let user_id = if stored_id.is_empty() {
        UNKNOWN
    } else {
        stored_id
    };
    denial::admin_only_route(route, role, &mask_user_id(user_id))
}

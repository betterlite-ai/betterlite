use betterlite_routes::wildcard::route_matches_wildcard_pattern;

use crate::route_match::groups::get_actual_routes;
use crate::user_roles::UserRole;

use super::template::template_matches;

// auth_checks.allowed_routes_check: a team token whose config names no route list gets these two groups
const TEAM_FALLBACK_ROUTES: [&str; 2] = ["openai_routes", "info_routes"];

// only a proxy admin and a team role carry JWT route grants; every other role has none
#[must_use]
pub fn allowed_routes_check(
    role: UserRole,
    route: &str,
    admin_allowed_routes: &[String],
    team_allowed_routes: Option<&[String]>,
) -> bool {
    match role {
        UserRole::ProxyAdmin => admin_allowed_routes
            .iter()
            .any(|entry| allows_entry(entry, route)),
        UserRole::Team => match team_allowed_routes {
            Some(routes) => routes.iter().any(|entry| allows_entry(entry, route)),
            None => TEAM_FALLBACK_ROUTES
                .iter()
                .any(|entry| allows_entry(entry, route)),
        },
        UserRole::ProxyAdminViewer
        | UserRole::OrgAdmin
        | UserRole::InternalUser
        | UserRole::InternalUserViewer
        | UserRole::Customer => false,
    }
}

// _allowed_routes_check: a group name expands to Starlette templates; anything else is an exact route or a trailing-wildcard prefix
fn allows_entry(entry: &str, route: &str) -> bool {
    let routes = get_actual_routes(&[entry]);
    if routes.len() == 1 && routes.first().is_some_and(|only| only.as_str() == entry) {
        return route_matches_wildcard_pattern(route, entry);
    }
    routes
        .iter()
        .any(|template| template_matches(template, route))
}

use betterlite_models::model_registration::JsonObject;

use crate::management_routes::ORG_ADMIN_ONLY_ROUTES;
use crate::organization_route_policy::error::RoutePolicyError;
use crate::organization_route_policy::membership::{ORG_ADMIN, PROXY_ADMIN, RouteUser};
use crate::organization_route_policy::request::PassedOrganizationId;

pub const ORGANIZATION_CREATE_ROUTE: &str = "/organization/new";
pub const TEAM_CREATE_ROUTE: &str = "/team/new";
// LiteLLM renders a missing user_role as the text None, in the stored role it names as well
const NO_ROLE: &str = "None";

// organization_role_based_access_check: /organization/new, B867's org admin routes and /team/new are the only policies.
pub fn check_organization_role(
    user: Option<&RouteUser>,
    route: &str,
    request: &JsonObject,
) -> Result<(), RoutePolicyError> {
    let Some(user) = user else {
        return Ok(());
    };
    let proxy_admin = user.user_role.as_deref() == Some(PROXY_ADMIN);
    if route == ORGANIZATION_CREATE_ROUTE && !proxy_admin {
        return Err(RoutePolicyError::OrganizationCreateDenied {
            role: user.user_role.as_deref().unwrap_or(NO_ROLE).to_owned(),
        });
    }
    if proxy_admin {
        return Ok(());
    }
    if ORG_ADMIN_ONLY_ROUTES.contains(&route) {
        return check_org_admin_route(user, route, request);
    }
    if route == TEAM_CREATE_ROUTE {
        return check_team_create(user, route, request);
    }
    Ok(())
}

// the organization the request selects and the role the caller holds there, None when it selects none
fn selected_organization<'a>(
    user: &'a RouteUser,
    request: &JsonObject,
) -> Result<Option<(String, Option<&'a str>)>, RoutePolicyError> {
    let PassedOrganizationId::Text(organization_id) = PassedOrganizationId::of(request)? else {
        return Ok(None);
    };
    let role = user
        .organization_memberships
        .iter()
        .flatten()
        .rev()
        .find(|membership| membership.organization_id.as_deref() == Some(organization_id.as_str()))
        .and_then(|membership| membership.user_role.as_deref());
    Ok(Some((organization_id, role)))
}

// The org admin matrix: memberships must exist, the selected organization must be passed, and its stored role must be org_admin.
fn check_org_admin_route(
    user: &RouteUser,
    route: &str,
    request: &JsonObject,
) -> Result<(), RoutePolicyError> {
    if user.organization_memberships.is_none() {
        return Err(RoutePolicyError::NotAMember {
            route: route.to_owned(),
        });
    }
    let Some((organization_id, role)) = selected_organization(user, request)? else {
        return Err(RoutePolicyError::MissingOrganizationId);
    };
    if role == Some(ORG_ADMIN) {
        return Ok(());
    }
    let Some(stored) = role else {
        return Err(RoutePolicyError::NoRoleInOrganization { organization_id });
    };
    Err(RoutePolicyError::NotOrganizationAdmin {
        route: route.to_owned(),
        organization_id,
        role: stored.to_owned(),
    })
}

// /team/new: any membership at all makes organization_id required, and the selected organization must make the caller its org admin.
fn check_team_create(
    user: &RouteUser,
    route: &str,
    request: &JsonObject,
) -> Result<(), RoutePolicyError> {
    let info = user.organization_info();
    if user
        .organization_memberships
        .as_ref()
        .is_none_or(Vec::is_empty)
    {
        return Ok(());
    }
    match selected_organization(user, request)? {
        None => Err(RoutePolicyError::TeamCreateNeedsOrganization {
            organizations: info.organizations.join(", "),
        }),
        Some((_, Some(ORG_ADMIN))) => Ok(()),
        Some((organization_id, role)) => Err(RoutePolicyError::TeamCreateNeedsOrgAdmin {
            route: route.to_owned(),
            organization_id,
            role: role.unwrap_or(NO_ROLE).to_owned(),
        }),
    }
}

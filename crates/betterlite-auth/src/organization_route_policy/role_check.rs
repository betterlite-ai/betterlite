use betterlite_models::model_registration::JsonObject;

use crate::management_routes::ORG_ADMIN_ONLY_ROUTES;
use crate::organization_route_policy::error::RoutePolicyError;
use crate::organization_route_policy::membership::{ORG_ADMIN, RouteUser};
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
    if route == ORGANIZATION_CREATE_ROUTE && !user.is_proxy_admin() {
        return Err(RoutePolicyError::OrganizationCreateDenied {
            role: display_role(user.user_role.as_deref()),
        });
    }
    if user.is_proxy_admin() {
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

// The org admin matrix: memberships must exist, the selected organization must be passed, and its stored role must be org_admin.
fn check_org_admin_route(
    user: &RouteUser,
    route: &str,
    request: &JsonObject,
) -> Result<(), RoutePolicyError> {
    let info = user.organization_info();
    if user.organization_memberships.is_none() {
        return Err(RoutePolicyError::NotAMember {
            route: route.to_owned(),
        });
    }
    let passed = PassedOrganizationId::of(request)?;
    let PassedOrganizationId::Text(organization_id) = passed else {
        return Err(RoutePolicyError::MissingOrganizationId);
    };
    // mapping.get(id) answers None for an absent key and for a stored None role alike
    let Some(Some(role)) = info.roles.get(&organization_id) else {
        return Err(RoutePolicyError::NoRoleInOrganization { organization_id });
    };
    if role.as_str() == ORG_ADMIN {
        return Ok(());
    }
    Err(RoutePolicyError::NotOrganizationAdmin {
        route: route.to_owned(),
        organization_id,
        role: role.clone(),
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
    let passed = PassedOrganizationId::of(request)?;
    let PassedOrganizationId::Text(organization_id) = passed else {
        return Err(RoutePolicyError::TeamCreateNeedsOrganization {
            organizations: info.organizations.join(", "),
        });
    };
    let role = info.roles.get(&organization_id).and_then(Option::as_deref);
    if role == Some(ORG_ADMIN) {
        return Ok(());
    }
    Err(RoutePolicyError::TeamCreateNeedsOrgAdmin {
        route: route.to_owned(),
        organization_id,
        role: display_role(role),
    })
}

fn display_role(role: Option<&str>) -> String {
    role.unwrap_or(NO_ROLE).to_owned()
}

use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::organization_route_policy::membership::{ORG_ADMIN, RouteUser};

// _user_is_org_admin: every requested organization must be one the user administers, and a proxy admin has no shortcut.
#[must_use]
pub fn user_is_org_admin(request: &JsonObject, user: Option<&RouteUser>) -> bool {
    let Some(caller) = user else {
        return false;
    };
    let Some(memberships) = &caller.organization_memberships else {
        return false;
    };
    let mut candidates: Vec<&Json> = Vec::new();
    if let Some(singular) = request.get("organization_id")
        && !matches!(singular, Json::Null)
    {
        candidates.push(singular);
    }
    // organizations counts only as a list, so a string or an object asking for an organization is no candidate
    if let Some(Json::List(organizations)) = request.get("organizations") {
        candidates.extend(organizations);
    }
    if candidates.is_empty() {
        return false;
    }
    let admin_ids: BTreeSet<&str> = memberships
        .iter()
        .filter(|membership| membership.user_role.as_deref() == Some(ORG_ADMIN))
        .filter_map(|membership| membership.organization_id.as_deref())
        .collect();
    candidates.iter().all(|candidate| {
        let Json::Text(organization_id) = candidate else {
            return false;
        };
        admin_ids.contains(organization_id.as_str())
    })
}

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct OwnerScope(pub String);

#[must_use]
pub fn is_proxy_admin(principal: Option<&Principal>) -> bool {
    let Some(identity) = principal else {
        return false;
    };
    identity.roles.contains(&Role::PlatformAdmin)
}

#[must_use]
pub fn get_resource_owner_scopes(principal: Option<&Principal>) -> Vec<OwnerScope> {
    let Some(identity) = principal else {
        return Vec::new();
    };
    let mut scopes = Vec::new();
    let user = identity.user.as_ref().map(|user| user.id.as_str());
    // Legacy auth carries one active team; additional memberships do not widen ownership.
    let team = identity.teams.first().map(|team| team.id.as_str());
    let org = identity.organization.as_ref().map(|org| org.id.as_str());
    let candidates = [
        ("", user),
        ("user:", user),
        ("team:", team),
        ("org:", org),
        ("key:", identity.credential_ref.key_id.as_deref()),
        ("key:", identity.credential_ref.token_id.as_deref()),
    ];
    for (prefix, value) in candidates {
        if let Some(nonempty) = value.filter(|candidate| !candidate.is_empty()) {
            let scope = OwnerScope(format!("{prefix}{nonempty}"));
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
    }
    scopes
}

#[must_use]
pub fn get_primary_owner_scope(principal: Option<&Principal>) -> Option<OwnerScope> {
    let identity = principal?;
    let candidates = [
        ("", identity.user.as_ref().map(|user| user.id.as_str())),
        ("team:", identity.teams.first().map(|team| team.id.as_str())),
        (
            "org:",
            identity.organization.as_ref().map(|org| org.id.as_str()),
        ),
        ("key:", identity.credential_ref.key_id.as_deref()),
        ("key:", identity.credential_ref.token_id.as_deref()),
    ];
    candidates.into_iter().find_map(|(prefix, value)| {
        value
            .filter(|candidate| !candidate.is_empty())
            .map(|nonempty| OwnerScope(format!("{prefix}{nonempty}")))
    })
}

#[must_use]
pub fn user_can_access_resource_owner(owner: Option<&str>, principal: Option<&Principal>) -> bool {
    if principal.is_none() || is_proxy_admin(principal) {
        return true;
    }
    let Some(owner_scope) = owner else {
        return false;
    };
    get_resource_owner_scopes(principal)
        .iter()
        .any(|scope| scope.0 == owner_scope)
}

mod tests;

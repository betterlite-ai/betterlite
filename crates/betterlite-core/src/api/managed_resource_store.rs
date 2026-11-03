use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct OwnerPrincipal {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub org_id: Option<String>,
    #[serde(rename = "api_key")]
    pub hashed_api_key: Option<String>,
    pub token: Option<String>,
    pub user_role: Option<String>,
}

#[must_use]
pub fn is_proxy_admin(principal: Option<&OwnerPrincipal>) -> bool {
    let Some(identity) = principal else {
        return false;
    };
    let role = identity.user_role.as_deref();
    role == Some("proxy_admin")
}

#[must_use]
pub fn get_resource_owner_scopes(principal: Option<&OwnerPrincipal>) -> Vec<String> {
    let Some(identity) = principal else {
        return Vec::new();
    };
    let mut scopes = Vec::new();
    if let Some(user_id) = identity
        .user_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        add_scope(&mut scopes, user_id.to_owned());
        add_scope(&mut scopes, format!("user:{user_id}"));
    }
    if let Some(team_id) = identity
        .team_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        add_scope(&mut scopes, format!("team:{team_id}"));
    }
    if let Some(org_id) = identity.org_id.as_deref().filter(|value| !value.is_empty()) {
        add_scope(&mut scopes, format!("org:{org_id}"));
    }
    if let Some(hashed_api_key) = identity
        .hashed_api_key
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        add_scope(&mut scopes, format!("key:{hashed_api_key}"));
    }
    if let Some(token) = identity.token.as_deref().filter(|value| !value.is_empty()) {
        add_scope(&mut scopes, format!("key:{token}"));
    }
    scopes
}

fn add_scope(scopes: &mut Vec<String>, scope: String) {
    if !scopes.contains(&scope) {
        scopes.push(scope);
    }
}

#[must_use]
pub fn get_primary_owner_scope(principal: Option<&OwnerPrincipal>) -> Option<String> {
    let identity = principal?;
    identity
        .user_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            identity
                .team_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|value| format!("team:{value}"))
        })
        .or_else(|| {
            identity
                .org_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|value| format!("org:{value}"))
        })
        .or_else(|| {
            identity
                .hashed_api_key
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|value| format!("key:{value}"))
        })
        .or_else(|| {
            identity
                .token
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|value| format!("key:{value}"))
        })
}

#[must_use]
pub fn user_can_access_resource_owner(
    owner: Option<&str>,
    principal: Option<&OwnerPrincipal>,
) -> bool {
    let Some(identity) = principal else {
        return true;
    };
    if is_proxy_admin(Some(identity)) {
        return true;
    }
    let Some(owner_scope) = owner else {
        return false;
    };
    get_resource_owner_scopes(Some(identity))
        .iter()
        .any(|scope| scope == owner_scope)
}

mod tests;

use super::error::TeamSelectionError;

// the RBAC switch and the proxy-admin exemption validate_object_id reads
pub struct ObjectPolicy {
    pub enforce_rbac: bool,
    pub is_proxy_admin: bool,
}

// validate_object_id: spend tracking needs an identity whenever RBAC is enforced for a non-admin caller
pub const fn validate_object_id(
    user_id: Option<&str>,
    team_id: Option<&str>,
    policy: ObjectPolicy,
) -> Result<(), TeamSelectionError> {
    if policy.enforce_rbac && !policy.is_proxy_admin && user_id.is_none() && team_id.is_none() {
        return Err(TeamSelectionError::ObjectIdentityRequired);
    }
    Ok(())
}

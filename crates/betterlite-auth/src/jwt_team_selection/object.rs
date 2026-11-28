use super::error::TeamSelectionError;

// validate_object_id: spend tracking needs an identity whenever RBAC is enforced for a non-admin caller
pub fn validate_object_id(
    user_id: Option<&str>,
    team_id: Option<&str>,
    enforce_rbac: bool,
    is_proxy_admin: bool,
) -> Result<(), TeamSelectionError> {
    if enforce_rbac && !is_proxy_admin && user_id.is_none() && team_id.is_none() {
        return Err(TeamSelectionError::ObjectIdentityRequired);
    }
    Ok(())
}

use betterlite_auth::user_roles::UserRole;
use serde::Deserialize;
use serde::de::Error as _;

use crate::proxy_request_contracts::{ContractError, Fields};

// The sentences B8411's user requests raise, reused by the typed rows that inherit their validators.
pub(crate) const USER_SELECTOR: &str = "Either user id or user email must be provided";
pub(crate) const PASSWORD_UNSUPPORTED: &str = "password cannot be set via /user/new. Users set their own password through an invitation link (POST /invitation/new).";

pub(crate) fn check_count(field: &str, count: usize, maximum: usize) -> Result<(), ContractError> {
    if count == 0 || count > maximum {
        return Err(ContractError::from(format!(
            "{field} must hold between 1 and {maximum} values"
        )));
    }
    Ok(())
}

pub(crate) fn reject_unknown_fields(
    fields: &Fields,
    allowed: &[&str],
) -> Result<(), ContractError> {
    for name in fields.keys() {
        if !allowed.contains(&name.as_str()) {
            return Err(ContractError::from(format!(
                "extra inputs are not permitted: `{name}`"
            )));
        }
    }
    Ok(())
}

// the mapping's value type is not optional, so an explicit null fails where an absent key is allowed
pub(crate) fn present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let value = Option::<T>::deserialize(deserializer)?;
    let inner = value.ok_or_else(|| D::Error::custom("null is not a mapping value"))?;
    Ok(Some(inner))
}

// LiteLLM's two user requests accept four of the seven roles; the literal rejects the other three.
pub(crate) fn check_user_role(role: Option<UserRole>) -> Result<(), ContractError> {
    match role {
        None
        | Some(
            UserRole::ProxyAdmin
            | UserRole::ProxyAdminViewer
            | UserRole::InternalUser
            | UserRole::InternalUserViewer,
        ) => Ok(()),
        Some(unsupported @ (UserRole::OrgAdmin | UserRole::Team | UserRole::Customer)) => Err(
            ContractError::from(format!("user_role {unsupported} is not allowed")),
        ),
    }
}

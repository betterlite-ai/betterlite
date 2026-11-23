use serde::Deserialize;

use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::user_roles::UserRole;

pub mod lifecycle;

mod tests;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum IndexOperation {
    Create,
    Delete,
    Update,
    List,
}

#[derive(Debug, thiserror::Error)]
pub enum IndexAdminError {
    #[error("Only proxy admins can {0} vector store indexes. Contact your LiteLLM administrator.")]
    NotProxyAdmin(IndexOperation),
}

#[must_use]
pub const fn auth_is_proxy_admin(auth: &UserAPIKeyAuth) -> bool {
    match auth.extra.user_role {
        Some(UserRole::ProxyAdmin) => true,
        Some(
            UserRole::ProxyAdminViewer
            | UserRole::OrgAdmin
            | UserRole::InternalUser
            | UserRole::InternalUserViewer
            | UserRole::Team
            | UserRole::Customer,
        )
        | None => false,
    }
}

pub const fn require_index_admin(
    auth: &UserAPIKeyAuth,
    operation: IndexOperation,
) -> Result<(), IndexAdminError> {
    if auth_is_proxy_admin(auth) {
        return Ok(());
    }
    Err(IndexAdminError::NotProxyAdmin(operation))
}

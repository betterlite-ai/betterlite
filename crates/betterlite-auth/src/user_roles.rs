use serde::{Deserialize, Serialize};

mod tests;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    ProxyAdmin,
    ProxyAdminViewer,
    OrgAdmin,
    InternalUser,
    InternalUserViewer,
    Team,
    Customer,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserApiKeyAuth {
    pub user_role: Option<UserRole>,
}

#[must_use]
pub const fn user_api_key_has_admin_view(api_key: &UserApiKeyAuth) -> bool {
    match api_key.user_role {
        Some(UserRole::ProxyAdmin | UserRole::ProxyAdminViewer) => true,
        Some(
            UserRole::OrgAdmin
            | UserRole::InternalUser
            | UserRole::InternalUserViewer
            | UserRole::Team
            | UserRole::Customer,
        )
        | None => false,
    }
}

use serde::{Deserialize, Serialize};

mod tests;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum UserRole {
    ProxyAdmin,
    ProxyAdminViewer,
    OrgAdmin,
    InternalUser,
    InternalUserViewer,
    Team,
    Customer,
}

impl UserRole {
    #[must_use]
    pub const fn description_and_ui_label(self) -> (&'static str, &'static str) {
        match self {
            Self::ProxyAdmin => (
                "admin over litellm proxy, has all permissions",
                "Admin (All Permissions)",
            ),
            Self::ProxyAdminViewer => ("view all keys, view all spend", "Admin (View Only)"),
            Self::OrgAdmin => ("", ""),
            Self::InternalUser => (
                "view/create/delete their own keys, view their own spend",
                "Internal User (Create/Delete/View)",
            ),
            Self::InternalUserViewer => (
                "view their own keys, view their own spend",
                "Internal User (View Only)",
            ),
            Self::Team => ("team scope used for JWT auth", "Team"),
            Self::Customer => ("customer", "Customer"),
        }
    }

    #[must_use]
    pub const fn is_internal_user_role(self) -> bool {
        match self {
            Self::InternalUser | Self::InternalUserViewer => true,
            Self::ProxyAdmin
            | Self::ProxyAdminViewer
            | Self::OrgAdmin
            | Self::Team
            | Self::Customer => false,
        }
    }
}

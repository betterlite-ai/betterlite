use axum::http::StatusCode;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::org_admin_keys::Settings as OrgAdminKeySettings;
use serde::Serialize;

mod tests;

pub const KEY_GENERATION_DISABLED: &str =
    "key generation is disabled for org admins. Contact your proxy admin.";

// LiteLLM raises HTTPException(403, {"error": ...}) here; B055 turns this detail into that response
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ErrorDetail {
    pub error: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OrgAdminKeysError {
    #[error("{}", KEY_GENERATION_DISABLED)]
    KeyGenerationDisabled,
}

impl OrgAdminKeysError {
    #[must_use]
    pub const fn status_code(self) -> StatusCode {
        match self {
            Self::KeyGenerationDisabled => StatusCode::FORBIDDEN,
        }
    }

    #[must_use]
    pub const fn detail(self) -> ErrorDetail {
        match self {
            Self::KeyGenerationDisabled => ErrorDetail {
                error: KEY_GENERATION_DISABLED,
            },
        }
    }
}

// only the exact org_admin role is gated: an org membership on any other global role stays with the caller's own key checks
pub const fn check_org_admin_can_generate_keys(
    role: UserRole,
    settings: &OrgAdminKeySettings,
) -> Result<(), OrgAdminKeysError> {
    match role {
        UserRole::OrgAdmin => {
            if settings.disable_key_generate_for_org_admin {
                Err(OrgAdminKeysError::KeyGenerationDisabled)
            } else {
                Ok(())
            }
        }
        UserRole::ProxyAdmin
        | UserRole::ProxyAdminViewer
        | UserRole::InternalUser
        | UserRole::InternalUserViewer
        | UserRole::Team
        | UserRole::Customer => Ok(()),
    }
}

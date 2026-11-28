use betterlite_config::sections::auth_user_lookup::RoleBasedPermissions;
use betterlite_config::sections::jwt_settings;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// litellm_jwtauth and general_settings.role_permissions, read by the JWT permission stage
pub struct Slot {
    pub settings: jwt_settings::Settings,
    pub role_permissions: Option<Vec<RoleBasedPermissions>>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            settings: config.sections.jwt_settings.clone(),
            role_permissions: config.sections.auth_user_lookup.role_permissions.clone(),
        }
    }
}

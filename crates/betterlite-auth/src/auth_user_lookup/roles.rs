use betterlite_config::sections::auth_user_lookup::{RbacRole, RoleBasedPermissions};
use betterlite_db::budget_permission_rows::JsonData;
use serde_json::Error as JsonError;

// auth_checks.py's ROLE_BASED_PERMISSIONS_ADAPTER: the whole list validates before any role is selected
#[derive(Debug, thiserror::Error)]
#[error("role_permissions is not a valid list of role permissions")]
pub struct RolePermissionError(#[source] JsonError);

#[derive(Clone, Debug, Default)]
pub struct RolePermissions {
    configured: Option<Vec<RoleBasedPermissions>>,
}

impl RolePermissions {
    #[must_use]
    pub fn configured(configured: Option<Vec<RoleBasedPermissions>>) -> Self {
        Self { configured }
    }

    pub fn from_json(configured: &JsonData) -> Result<Self, RolePermissionError> {
        if matches!(configured, JsonData::Null) {
            return Ok(Self::default());
        }
        let parsed = serde_json::to_value(configured)
            .and_then(serde_json::from_value::<Vec<RoleBasedPermissions>>)
            .map_err(RolePermissionError)?;
        Ok(Self {
            configured: Some(parsed),
        })
    }

    // get_role_based_models / get_role_based_routes: the first entry naming the role decides, even when empty
    #[must_use]
    pub fn models(&self, role: RbacRole) -> Option<&[String]> {
        self.selected(role)
            .and_then(|entry| entry.models.as_deref())
    }

    #[must_use]
    pub fn routes(&self, role: RbacRole) -> Option<&[String]> {
        self.selected(role)
            .and_then(|entry| entry.routes.as_deref())
    }

    fn selected(&self, role: RbacRole) -> Option<&RoleBasedPermissions> {
        self.configured
            .as_ref()?
            .iter()
            .find(|entry| entry.role == role)
    }
}

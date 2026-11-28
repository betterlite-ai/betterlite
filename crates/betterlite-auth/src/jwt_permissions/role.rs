use betterlite_config::sections::auth_user_lookup::{RbacRole, RoleBasedPermissions};
use betterlite_config::sections::jwt_settings::Settings;
use betterlite_db::budget_permission_rows::JsonData;

use crate::auth_errors::conversion::AuthFailure;
use crate::auth_model_patterns::denial::model_denied_client_message;
use crate::auth_route_policy::jwt::allowed_routes_check;
use crate::auth_user_lookup::roles::{RolePermissionError, RolePermissions};
use crate::user_roles::UserRole;

const FORBIDDEN: u16 = 403;
const UNMATCHED_ROLE: &str = "Unmatched token passed in. enforce_rbac is set to True. Token must \
                              belong to a proxy admin, team, or user.";

#[derive(Clone, Copy)]
enum RoleList {
    Models,
    Routes,
}

#[derive(Clone, Copy)]
pub enum RoleCheck<'request> {
    Model(Option<&'request str>),
    Route(&'request str),
}

// get_role_based_routes and get_role_based_models read one list off the first entry naming the role, and a null list leaves that dimension open
fn role_list(
    permissions: &RolePermissions,
    rbac_role: RbacRole,
    list: RoleList,
) -> Option<&[String]> {
    let entry = permissions.selected(rbac_role)?;
    match list {
        RoleList::Models => entry.models.as_deref(),
        RoleList::Routes => entry.routes.as_deref(),
    }
}

// can_rbac_role_call_route then can_rbac_role_call_model: a null list or a missing model leaves that dimension open, models match by literal membership, and route names still expand their groups
pub fn can_rbac_role_call(
    permissions: &RolePermissions,
    rbac_role: RbacRole,
    check: RoleCheck<'_>,
) -> Result<(), AuthFailure> {
    match check {
        RoleCheck::Route(route) => {
            let Some(allowed_routes) = role_list(permissions, rbac_role, RoleList::Routes) else {
                return Ok(());
            };
            if allowed_routes_check(UserRole::ProxyAdmin, route, allowed_routes, None) {
                return Ok(());
            }
            Err(AuthFailure::Http {
                detail: format!(
                    "Role={rbac_role:?} not allowed to call route={route}. Allowed routes={allowed_routes:?}"
                ),
                param: None,
                status_code: FORBIDDEN,
                invalid_virtual_key: false,
            })
        }
        RoleCheck::Model(model) => {
            let Some(allowed_models) = role_list(permissions, rbac_role, RoleList::Models) else {
                return Ok(());
            };
            let Some(model) = model else {
                return Ok(());
            };
            if allowed_models
                .iter()
                .any(|allowed| allowed.as_str() == model)
            {
                return Ok(());
            }
            Err(AuthFailure::ModelAccessDenied {
                detail: model_denied_client_message(model),
                internal_message: format!(
                    "Role={rbac_role:?} not allowed to call model={model}. Allowed models={allowed_models:?}"
                ),
                status_code: FORBIDDEN,
            })
        }
    }
}

// enforce_rbac turns the role mappings on, and the model check runs before the route check
pub fn check_rbac_role(
    settings: &Settings,
    permissions: &RolePermissions,
    rbac_role: Option<RbacRole>,
    model: Option<&str>,
    route: &str,
) -> Result<(), AuthFailure> {
    if !settings.rbac.enforce_rbac {
        return Ok(());
    }
    let Some(rbac_role) = rbac_role else {
        return Err(AuthFailure::Http {
            detail: UNMATCHED_ROLE.to_owned(),
            param: None,
            status_code: FORBIDDEN,
            invalid_virtual_key: false,
        });
    };
    can_rbac_role_call(permissions, rbac_role, RoleCheck::Model(model))?;
    can_rbac_role_call(permissions, rbac_role, RoleCheck::Route(route))
}

#[derive(Debug, thiserror::Error)]
pub enum RolePermissionSetupError {
    #[error("role_permissions could not be serialized")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Permissions(#[from] RolePermissionError),
}

// the config section validated this list already; B736 reads general_settings.role_permissions as JSON
pub fn role_permissions(
    configured: Option<&[RoleBasedPermissions]>,
) -> Result<RolePermissions, RolePermissionSetupError> {
    let Some(entries) = configured else {
        return Ok(RolePermissions::default());
    };
    let json = serde_json::from_value::<JsonData>(serde_json::to_value(entries)?)?;
    Ok(RolePermissions::from_json(&json)?)
}

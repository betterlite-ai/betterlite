use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_auth::resolved_key::auth::user_api_key_has_admin_view;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;

// the user_api_key_dict fields the organization handlers read, taken once from the key's serde form
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgCaller {
    pub user_id: Option<String>,
    pub user_role: Option<UserRole>,
    pub models: Vec<JsonData>,
    pub admin_view: bool,
}

#[derive(Debug, Deserialize)]
struct KeyView {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_role: Option<UserRole>,
    #[serde(default)]
    models: Option<Vec<JsonData>>,
}

impl OrgCaller {
    pub fn from_principal(principal: &Principal) -> Result<Self, serde_json::Error> {
        let key = principal.source_key.0.as_ref();
        let stored = key
            .map(|record| serde_json::from_value::<KeyView>(serde_json::to_value(record)?))
            .transpose()?;
        let user_id = stored
            .as_ref()
            .and_then(|view| view.user_id.clone())
            .or_else(|| principal.user.as_ref().map(|user| user.id.clone()));
        let (user_role, models, admin_view) = match (stored, key) {
            (Some(view), Some(record)) => (
                view.user_role,
                view.models.unwrap_or_default(),
                user_api_key_has_admin_view(record),
            ),
            (None, None) => {
                let role = role_from_roles(&principal.roles);
                let admin_view = matches!(
                    role,
                    Some(UserRole::ProxyAdmin | UserRole::ProxyAdminViewer)
                );
                (role, Vec::new(), admin_view)
            }
            (Some(view), None) => (view.user_role, view.models.unwrap_or_default(), false),
            (None, Some(_)) => (None, Vec::new(), false),
        };
        Ok(Self {
            user_id,
            user_role,
            models,
            admin_view,
        })
    }

    // the role text the 401 renders, Python's None when the key carries no role
    #[must_use]
    pub fn role_text(&self) -> String {
        self.user_role
            .map_or_else(|| "None".to_owned(), |role| role.to_string())
    }
}

fn role_from_roles(roles: &[Role]) -> Option<UserRole> {
    if roles.contains(&Role::PlatformAdmin) {
        return Some(UserRole::ProxyAdmin);
    }
    if roles.contains(&Role::PlatformViewer) {
        return Some(UserRole::ProxyAdminViewer);
    }
    if roles.contains(&Role::OrgAdmin) {
        return Some(UserRole::OrgAdmin);
    }
    None
}

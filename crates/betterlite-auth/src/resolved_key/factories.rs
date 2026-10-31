use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use super::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use crate::user_roles::UserRole;

mod tests;

const HEALTH_CHECK_NAME: &str = "litellm-internal-health-check";
const JOBS_NAME: &str = "litellm_internal_jobs";

fn string_fields(entries: &[(&str, &str)]) -> BTreeMap<String, JsonData> {
    let mut fields = BTreeMap::new();
    for (key, value) in entries {
        fields.insert((*key).to_owned(), JsonData::String((*value).to_owned()));
    }
    fields
}

pub enum SystemPrincipal {
    HealthCheck,
    Jobs,
}

pub fn system_principal(kind: &SystemPrincipal) -> Result<UserAPIKeyAuth, AuthError> {
    let fields = match kind {
        SystemPrincipal::HealthCheck => string_fields(&[
            ("api_key", HEALTH_CHECK_NAME),
            ("team_id", HEALTH_CHECK_NAME),
            ("key_alias", HEALTH_CHECK_NAME),
            ("team_alias", HEALTH_CHECK_NAME),
        ]),
        SystemPrincipal::Jobs => {
            let mut fields = string_fields(&[
                ("api_key", JOBS_NAME),
                ("key_alias", JOBS_NAME),
                ("team_id", "system"),
                ("team_alias", "system"),
                ("user_id", "system"),
            ]);
            let user_role = UserRole::ProxyAdmin.to_string();
            fields.insert("user_role".to_owned(), JsonData::String(user_role));
            fields
        }
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(fields))
}

#[must_use]
pub fn is_team_service_account(auth: &UserAPIKeyAuth) -> bool {
    match auth.view.token.metadata.0.get("service_account_id") {
        None | Some(JsonData::Null) => false,
        Some(_) => {
            auth.view.token.user_id.is_none()
                && auth.view.token.team_id.is_some()
                && !auth.view.token.metadata.0.is_empty()
        }
    }
}

#[must_use]
pub const fn user_api_key_has_admin_view(auth: &UserAPIKeyAuth) -> bool {
    match auth.extra.user_role {
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

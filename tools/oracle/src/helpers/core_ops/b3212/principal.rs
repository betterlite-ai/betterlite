use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use serde_json::json;

use super::PrincipalFields;
use betterlite_oracle_helper_api::error::HelperError;

pub(super) fn principal_of(fields: &PrincipalFields) -> Result<Principal, Box<HelperError>> {
    let user = fields.user_id.as_deref();
    let teams = match &fields.team_id {
        Some(id) => vec![json!({"id": id})],
        None => Vec::new(),
    };
    let roles = if fields.admin {
        vec![Role::PlatformAdmin]
    } else {
        Vec::new()
    };
    serde_json::from_value(json!({
        "principal_type": "human",
        "subject": user.unwrap_or("scripted"),
        "user": user.map(|id| json!({"id": id})),
        "organization": fields.org_id.as_ref().map(|id| json!({"id": id})),
        "teams": teams,
        "roles": roles,
        "auth_method": "api_key",
        "credential_ref": {"key_id": &fields.api_key, "token_id": &fields.token},
    }))
    .map_err(|error| super::invalid_input(error.to_string()))
}

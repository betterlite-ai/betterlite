use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::skills::store::NewSkill;
use serde_json::json;

use super::{NewSkillFields, PrincipalFields};

pub(super) fn principal_of(
    fields: &PrincipalFields,
) -> Result<Principal, Box<crate::helpers::HelperError>> {
    let mut teams = Vec::new();
    if let Some(id) = &fields.team_id {
        teams.push(json!({"id": id}));
    }
    let roles = if fields.admin {
        json!(["platform_admin"])
    } else {
        json!([])
    };
    let value = json!({
        "principal_type": "human",
        "subject": fields.user_id.as_deref().unwrap_or("scripted"),
        "user": fields.user_id.as_ref().map(|id| json!({"id": id})),
        "organization": fields.org_id.as_ref().map(|id| json!({"id": id})),
        "teams": teams,
        "roles": roles,
        "auth_method": "api_key",
        "credential_ref": {"key_id": &fields.api_key, "token_id": &fields.token},
    });
    serde_json::from_value(value).map_err(|error| super::invalid_input(error.to_string()))
}

fn decode_content(text: Option<&str>) -> Result<Option<Vec<u8>>, Box<crate::helpers::HelperError>> {
    let Some(text) = text else {
        return Ok(None);
    };
    STANDARD
        .decode(text)
        .map(Some)
        .map_err(|error| super::invalid_input(error.to_string()))
}

pub(super) fn new_skill(
    fields: &NewSkillFields,
) -> Result<NewSkill, Box<crate::helpers::HelperError>> {
    Ok(NewSkill {
        display_title: fields.display_title.clone(),
        description: fields.description.clone(),
        instructions: fields.instructions.clone(),
        metadata: fields.metadata.clone(),
        file_content: decode_content(fields.file_content.as_deref())?,
        file_name: fields.file_name.clone(),
        file_type: fields.file_type.clone(),
    })
}

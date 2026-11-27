use betterlite_models::controls::json_value::JsonValue;
use thiserror::Error;

use crate::oauth_tokens::TypedValue;

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("the OAuth token response is not a JSON object")]
    NotAnObject,
    #[error("the OAuth token response field {field} is not a string")]
    NotAString { field: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtractedIdentity {
    pub user_id: Option<String>,
    pub user_role: Option<String>,
    pub team_id: Option<String>,
}

// _extract_user_info reads direct dictionary keys, so a configured name keeps its dots as literal text
pub fn extract_user_info(
    document: &TypedValue,
    user_id_field_name: &str,
    user_role_field_name: &str,
    user_team_id_field_name: &str,
) -> Result<ExtractedIdentity, ExtractError> {
    let JsonValue::Object(fields) = document else {
        return Err(ExtractError::NotAnObject);
    };
    Ok(ExtractedIdentity {
        user_id: field_text(fields.get(user_id_field_name), user_id_field_name)?,
        user_role: field_text(fields.get(user_role_field_name), user_role_field_name)?,
        team_id: field_text(fields.get(user_team_id_field_name), user_team_id_field_name)?,
    })
}

fn field_text(value: Option<&JsonValue>, name: &str) -> Result<Option<String>, ExtractError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(text)) => Ok(Some(text.clone())),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(ExtractError::NotAString {
            field: name.to_owned(),
        }),
    }
}

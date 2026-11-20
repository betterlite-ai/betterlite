use std::collections::BTreeMap;

use serde::Serialize;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::DeletedVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;

use crate::key_query_plan::KeyQueryPlanError;
use crate::key_query_plan::plan::KeyTable;

pub type KeyRow = BTreeMap<String, JsonData>;

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum KeyListEntry {
    Token(String),
    Auth(Box<UserAPIKeyAuth>),
    Deleted(Box<DeletedVerificationToken>),
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyListResponseObject {
    pub keys: Vec<KeyListEntry>,
    pub total_count: Option<i64>,
    pub current_page: Option<i64>,
    pub total_pages: Option<i64>,
}

pub fn entry_for_row(
    row: KeyRow,
    table: KeyTable,
    full_object: bool,
) -> Result<KeyListEntry, KeyQueryPlanError> {
    if full_object {
        return Ok(match table {
            KeyTable::Live => {
                let auth = UserAPIKeyAuth::construct(AuthInput::Dict(row))?;
                KeyListEntry::Auth(Box::new(auth))
            }
            KeyTable::Deleted => KeyListEntry::Deleted(Box::new(deleted_row(&row)?)),
        });
    }
    let Some(JsonData::String(token)) = row.get("token") else {
        return Err(KeyQueryPlanError::MissingToken);
    };
    Ok(KeyListEntry::Token(token.clone()))
}

// the archived table is dumped through its own model, which keeps the deletion metadata
fn deleted_row(row: &KeyRow) -> Result<DeletedVerificationToken, KeyQueryPlanError> {
    let document =
        serde_json::to_string(&JsonData::Object(row.clone())).map_err(KeyQueryPlanError::Row)?;
    serde_json::from_str(&document).map_err(KeyQueryPlanError::Row)
}

// model_dump for the owner and the three creator fields LiteLLM copies out of the user row
pub fn attach_user_expansion(
    row: &mut KeyRow,
    users: &BTreeMap<String, LiteLLMUserTable>,
) -> Result<(), KeyQueryPlanError> {
    if let Some(user_id) = non_empty_text(row.get("user_id"))
        && let Some(user) = users.get(user_id)
    {
        let document = serde_json::to_string(user).map_err(KeyQueryPlanError::Row)?;
        let value = serde_json::from_str(&document).map_err(KeyQueryPlanError::Row)?;
        row.insert(String::from("user"), value);
    }
    if let Some(user_id) = non_empty_text(row.get("created_by"))
        && let Some(user) = users.get(user_id)
    {
        let mut fields = BTreeMap::new();
        fields.insert(
            String::from("user_id"),
            JsonData::String(user.user_id.clone()),
        );
        fields.insert(String::from("user_email"), optional_text(&user.user_email));
        fields.insert(String::from("user_alias"), optional_text(&user.user_alias));
        row.insert(String::from("created_by_user"), JsonData::Object(fields));
    }
    Ok(())
}

fn non_empty_text(value: Option<&JsonData>) -> Option<&str> {
    let Some(JsonData::String(text)) = value else {
        return None;
    };
    Some(text.as_str()).filter(|entry| !entry.is_empty())
}

fn optional_text(value: &Option<String>) -> JsonData {
    match value {
        Some(text) => JsonData::String(text.clone()),
        None => JsonData::Null,
    }
}

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use serde::{Deserialize, Serialize};

use crate::proxy_request_contracts::user::{UpdateUserFields, UpdateUserRequest};
use crate::proxy_request_contracts::{ContractError, Fields, require_object};
use crate::user_contracts::wire::wrong_type;

const SELECTION_MISSING: &str = "Must specify either 'users' for individual updates or 'all_users=True' with 'user_updates' for bulk updates";
const SELECTION_CONFLICT: &str =
    "Cannot specify both 'users' and 'all_users=True'. Choose one approach.";

#[derive(Debug, Clone)]
pub struct BulkUpdateUserRequest {
    pub users: Option<Vec<UpdateUserRequest>>,
    pub all_users: Option<bool>,
    pub user_updates: Option<UpdateUserFields>,
}

impl BulkUpdateUserRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request = Self {
            users: decode_users(&fields)?,
            all_users: decode_all_users(&fields)?,
            user_updates: decode_user_updates(&fields)?,
        };
        // pydantic's field validator runs only for a supplied user_updates, explicit null included
        if fields.contains_key("user_updates") {
            request.check_selection()?;
        }
        Ok(request)
    }

    fn check_selection(&self) -> Result<(), ContractError> {
        let any_users = self.users.as_ref().is_some_and(|users| !users.is_empty());
        let all_users = self.all_users == Some(true);
        if !any_users && !(all_users && self.user_updates.is_some()) {
            return Err(ContractError::from(SELECTION_MISSING));
        }
        if any_users && all_users {
            return Err(ContractError::from(SELECTION_CONFLICT));
        }
        Ok(())
    }
}

fn decode_users(fields: &Fields) -> Result<Option<Vec<UpdateUserRequest>>, ContractError> {
    let Some(found) = fields.get("users") else {
        return Ok(None);
    };
    let JsonData::Array(items) = found else {
        if matches!(found, JsonData::Null) {
            return Ok(None);
        }
        return Err(wrong_type("users"));
    };
    let mut users = Vec::with_capacity(items.len());
    for item in items {
        let document: JsonValue = serde_json::to_value(item)?;
        users.push(UpdateUserRequest::from_value(&document)?);
    }
    Ok(Some(users))
}

fn decode_all_users(fields: &Fields) -> Result<Option<bool>, ContractError> {
    match fields.get("all_users") {
        None => Ok(Some(false)),
        Some(JsonData::Null) => Ok(None),
        Some(JsonData::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(wrong_type("all_users")),
    }
}

fn decode_user_updates(fields: &Fields) -> Result<Option<UpdateUserFields>, ContractError> {
    let Some(found) = fields.get("user_updates") else {
        return Ok(None);
    };
    match found {
        JsonData::Null => Ok(None),
        JsonData::Object(_) => {
            let document: JsonValue = serde_json::to_value(found)?;
            Ok(Some(UpdateUserFields::from_value(&document)?))
        }
        _ => Err(wrong_type("user_updates")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserUpdateResult {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_email: Option<String>,
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub updated_user: Option<Payload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkUpdateUserResponse {
    pub results: Vec<UserUpdateResult>,
    pub total_requested: i64,
    pub successful_updates: i64,
    pub failed_updates: i64,
}

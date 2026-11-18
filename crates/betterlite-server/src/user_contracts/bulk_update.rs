use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use serde::de::Error as _;
use serde::{Deserialize, Serialize};

use crate::proxy_request_contracts::user::{UpdateUserFields, UpdateUserRequest};
use crate::proxy_request_contracts::{ContractError, Fields, decode, require_object};

const SELECTION_MISSING: &str = "Must specify either 'users' for individual updates or 'all_users=True' with 'user_updates' for bulk updates";
const SELECTION_CONFLICT: &str =
    "Cannot specify both 'users' and 'all_users=True'. Choose one approach.";

// The one scalar this wrapper decodes itself; each nested record keeps B8411's own request decoder.
#[derive(Deserialize)]
struct AllUsersWire {
    #[serde(default)]
    all_users: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct BulkUpdateUserRequest {
    pub users: Option<Vec<UpdateUserRequest>>,
    pub all_users: Option<bool>,
    pub user_updates: Option<UpdateUserFields>,
}

// Each row is a whole UpdateUserRequest, so B8411's blank-budget, selector, role and fields-set checks all run.
fn decode_rows(fields: &Fields) -> Result<Option<Vec<UpdateUserRequest>>, ContractError> {
    let Some(raw) = fields.get("users") else {
        return Ok(None);
    };
    if matches!(raw, JsonData::Null) {
        return Ok(None);
    }
    let JsonData::Array(rows) = raw else {
        return Err(ContractError::Decode {
            source: serde_json::Error::custom("users is not a JSON array"),
        });
    };
    let mut requests = Vec::with_capacity(rows.len());
    for row in rows {
        let value = serde_json::to_value(row).map_err(ContractError::from)?;
        requests.push(UpdateUserRequest::from_value(&value)?);
    }
    Ok(Some(requests))
}

// UpdateUserRequestNoUserIDorEmail has no selector, so the updates model runs the same decoder without one.
fn decode_updates(fields: &Fields) -> Result<Option<UpdateUserFields>, ContractError> {
    let Some(raw) = fields.get("user_updates") else {
        return Ok(None);
    };
    if matches!(raw, JsonData::Null) {
        return Ok(None);
    }
    let value = serde_json::to_value(raw).map_err(ContractError::from)?;
    Ok(Some(UpdateUserFields::from_value(&value)?))
}

impl BulkUpdateUserRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        // pydantic's field validator runs only for a supplied user_updates, explicit null included
        let supplied_updates = fields.contains_key("user_updates");
        let users = decode_rows(&fields)?;
        let all_users = if fields.contains_key("all_users") {
            decode::<AllUsersWire>(&fields)?.all_users
        } else {
            Some(false)
        };
        let request = Self {
            users,
            all_users,
            user_updates: decode_updates(&fields)?,
        };
        if supplied_updates {
            request.check_selection()?;
        }
        Ok(request)
    }

    fn check_selection(&self) -> Result<(), ContractError> {
        let any_users = self.users.as_ref().is_some_and(|users| !users.is_empty());
        let selected_all = self.all_users == Some(true) && self.user_updates.is_some();
        if !(any_users || selected_all) {
            return Err(ContractError::from(SELECTION_MISSING));
        }
        if any_users && self.all_users == Some(true) {
            return Err(ContractError::from(SELECTION_CONFLICT));
        }
        Ok(())
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

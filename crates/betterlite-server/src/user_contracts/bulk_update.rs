use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use serde::{Deserialize, Serialize};

use crate::proxy_request_contracts::user::{UpdateUserFields, UpdateUserRequest};
use crate::proxy_request_contracts::{ContractError, decode, require_object};

const SELECTION_MISSING: &str = "Must specify either 'users' for individual updates or 'all_users=True' with 'user_updates' for bulk updates";
const SELECTION_CONFLICT: &str =
    "Cannot specify both 'users' and 'all_users=True'. Choose one approach.";

// One serde record for the whole body; the nested records decode through B8411's own from_value below.
#[derive(Deserialize)]
struct BulkUpdateUserWire {
    #[serde(default)]
    users: Option<Vec<JsonData>>,
    #[serde(default)]
    all_users: Option<bool>,
    #[serde(default)]
    user_updates: Option<JsonData>,
}

#[derive(Debug, Clone)]
pub struct BulkUpdateUserRequest {
    pub users: Option<Vec<UpdateUserRequest>>,
    pub all_users: Option<bool>,
    pub user_updates: Option<UpdateUserFields>,
}

impl BulkUpdateUserRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        // pydantic's field validator runs only for a supplied user_updates, explicit null included
        let supplied_updates = fields.contains_key("user_updates");
        let wire: BulkUpdateUserWire = decode(&fields)?;
        let users: Option<Vec<UpdateUserRequest>> = wire
            .users
            .as_ref()
            .map(|rows| {
                rows.iter()
                    .map(|row| {
                        let document = serde_json::to_value(row).map_err(ContractError::from)?;
                        UpdateUserRequest::from_value(&document)
                    })
                    .collect::<Result<Vec<_>, ContractError>>()
            })
            .transpose()?;
        let user_updates: Option<UpdateUserFields> = wire
            .user_updates
            .as_ref()
            .map(|update| {
                let document = serde_json::to_value(update).map_err(ContractError::from)?;
                UpdateUserFields::from_value(&document)
            })
            .transpose()?;
        // pydantic defaults only an absent all_users, so an explicit null stays null
        let all_users = if fields.contains_key("all_users") {
            wire.all_users
        } else {
            Some(false)
        };
        let request = Self {
            users,
            all_users,
            user_updates,
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

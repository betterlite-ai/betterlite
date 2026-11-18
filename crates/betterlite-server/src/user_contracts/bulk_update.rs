use betterlite_config::sections::settings::JsonValue;
use betterlite_db::identity_rows::Payload;
use serde::{Deserialize, Serialize};

use crate::proxy_request_contracts::ContractError;
use crate::proxy_request_contracts::user::{UpdateUserFields, UpdateUserRequest};
use crate::user_contracts::wire::wrong_type;

const SELECTION_MISSING: &str = "Must specify either 'users' for individual updates or 'all_users=True' with 'user_updates' for bulk updates";
const SELECTION_CONFLICT: &str =
    "Cannot specify both 'users' and 'all_users=True'. Choose one approach.";

// The wire record is a typed serde view; the model fields keep pydantic's absent, null and value states.
#[derive(Deserialize)]
struct BulkUpdateUserWire {
    #[serde(default)]
    users: Option<Vec<JsonValue>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    all_users: Option<Option<bool>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    user_updates: Option<Option<JsonValue>>,
}

#[derive(Debug, Clone)]
pub struct BulkUpdateUserRequest {
    pub users: Option<Vec<UpdateUserRequest>>,
    pub all_users: Option<bool>,
    pub user_updates: Option<UpdateUserFields>,
}

impl BulkUpdateUserRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let wire: BulkUpdateUserWire = serde_json::from_value(value.clone())?;
        let supplied_updates = wire.user_updates.is_some();
        let users = match wire.users {
            Some(items) => Some(
                items
                    .iter()
                    .map(UpdateUserRequest::from_value)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            None => None,
        };
        let user_updates = match wire.user_updates {
            Some(Some(value @ JsonValue::Object(_))) => Some(UpdateUserFields::from_value(&value)?),
            Some(Some(_)) => return Err(wrong_type("user_updates")),
            Some(None) | None => None,
        };
        let request = Self {
            users,
            all_users: wire.all_users.unwrap_or(Some(false)),
            user_updates,
        };
        // pydantic's field validator runs only for a supplied user_updates, explicit null included
        if supplied_updates {
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

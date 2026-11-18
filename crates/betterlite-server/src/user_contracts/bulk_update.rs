use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::identity_rows::Payload;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::key_fields::base::GenerateRequestBase;
use crate::key_fields::decode::blank_to_null;
use crate::key_fields::values::Amount;
use crate::proxy_request_contracts::user::{USER_FIELDS, UpdateUserFields, UpdateUserRequest};
use crate::proxy_request_contracts::{
    ContractError, Fields, decode, require_object, supplied_fields,
};
use crate::user_contracts::wire::{USER_SELECTOR, check_user_role};

const SELECTION_MISSING: &str = "Must specify either 'users' for individual updates or 'all_users=True' with 'user_updates' for bulk updates";
const SELECTION_CONFLICT: &str =
    "Cannot specify both 'users' and 'all_users=True'. Choose one approach.";

// One /user/update body without its selectors: B8411's fields, decoded like UpdateUserFields::from_value.
#[derive(Deserialize)]
struct UpdateUserFieldsWire {
    #[serde(flatten)]
    base: GenerateRequestBase,
    #[serde(default)]
    password: Option<SecretString>,
    #[serde(default)]
    spend: Option<Amount>,
    #[serde(default)]
    metadata: Option<Payload>,
    #[serde(default)]
    user_alias: Option<String>,
    #[serde(default)]
    user_role: Option<UserRole>,
    #[serde(default)]
    max_budget: Option<Amount>,
}

impl UpdateUserFieldsWire {
    // model_fields_set keeps the six update keys the caller supplied, an explicit null included
    fn into_fields(self, fields: &Fields) -> UpdateUserFields {
        UpdateUserFields {
            base: self.base,
            password: self.password,
            spend: self.spend,
            metadata: self.metadata,
            user_alias: self.user_alias,
            user_role: self.user_role,
            max_budget: self.max_budget,
            supplied: supplied_fields(fields, &USER_FIELDS),
        }
    }
}

// One `users` row: the /user/update body with both selectors.
#[derive(Deserialize)]
struct UpdateUserRequestWire {
    #[serde(flatten)]
    fields: UpdateUserFieldsWire,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_email: Option<String>,
}

impl UpdateUserRequestWire {
    fn from_value(value: &JsonValue) -> Result<UpdateUserRequest, ContractError> {
        let mut fields = require_object(value)?;
        blank_to_null(&mut fields, &["max_budget"]);
        let wire: Self = decode(&fields)?;
        // the before-validator rejects a row whose selectors are both null
        if wire.user_id.is_none() && wire.user_email.is_none() {
            return Err(ContractError::from(USER_SELECTOR));
        }
        let decoded = wire.fields.into_fields(&fields);
        check_user_role(decoded.user_role)?;
        Ok(UpdateUserRequest {
            fields: decoded,
            user_id: wire.user_id,
            user_email: wire.user_email,
        })
    }
}

// The wrapper keeps pydantic's absent default and its explicit null as separate states.
#[derive(Deserialize)]
struct BulkUpdateUserWire {
    #[serde(default)]
    users: Option<Vec<JsonValue>>,
    #[serde(default)]
    all_users: Option<bool>,
    #[serde(default)]
    user_updates: Option<JsonValue>,
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
        let supplied_updates = fields.contains_key("user_updates");
        let wire: BulkUpdateUserWire = decode(&fields)?;
        let mut users = None;
        if let Some(rows) = wire.users {
            let mut requests = Vec::with_capacity(rows.len());
            for row in &rows {
                requests.push(UpdateUserRequestWire::from_value(row)?);
            }
            users = Some(requests);
        }
        let mut user_updates = None;
        if let Some(update) = wire.user_updates {
            let mut update_fields = require_object(&update)?;
            blank_to_null(&mut update_fields, &["max_budget"]);
            let update_wire: UpdateUserFieldsWire = decode(&update_fields)?;
            let decoded = update_wire.into_fields(&update_fields);
            check_user_role(decoded.user_role)?;
            user_updates = Some(decoded);
        }
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
        // pydantic's field validator runs only for a supplied user_updates, explicit null included
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

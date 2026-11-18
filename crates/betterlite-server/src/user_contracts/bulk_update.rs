use std::collections::BTreeSet;

use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::key_fields::base::GenerateRequestBase;
use crate::key_fields::decode::blank_to_null;
use crate::key_fields::values::Amount;
use crate::proxy_request_contracts::user::{UpdateUserFields, UpdateUserRequest};
use crate::proxy_request_contracts::{ContractError, Fields, decode, require_object};
use crate::user_contracts::wire::{USER_SELECTOR, check_user_role};

const SELECTION_MISSING: &str = "Must specify either 'users' for individual updates or 'all_users=True' with 'user_updates' for bulk updates";
const SELECTION_CONFLICT: &str =
    "Cannot specify both 'users' and 'all_users=True'. Choose one approach.";

// One /user/update body without its selectors: B8411's fields with pydantic's absent, null and value states.
#[derive(Deserialize)]
struct UpdateUserFieldsWire {
    #[serde(flatten)]
    base: GenerateRequestBase,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    password: Option<Option<SecretString>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    spend: Option<Option<Amount>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    metadata: Option<Option<Payload>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    user_alias: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    user_role: Option<Option<UserRole>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    max_budget: Option<Option<Amount>>,
}

impl UpdateUserFieldsWire {
    // model_fields_set keeps the six update keys the caller supplied, an explicit null included
    fn into_fields(self) -> UpdateUserFields {
        let supplied_now = [
            (self.password.is_some(), "password"),
            (self.spend.is_some(), "spend"),
            (self.metadata.is_some(), "metadata"),
            (self.user_alias.is_some(), "user_alias"),
            (self.user_role.is_some(), "user_role"),
            (self.max_budget.is_some(), "max_budget"),
        ];
        let mut supplied = BTreeSet::new();
        for (present, name) in supplied_now {
            if present {
                supplied.insert(name.to_owned());
            }
        }
        UpdateUserFields {
            base: self.base,
            password: self.password.flatten(),
            spend: self.spend.flatten(),
            metadata: self.metadata.flatten(),
            user_alias: self.user_alias.flatten(),
            user_role: self.user_role.flatten(),
            max_budget: self.max_budget.flatten(),
            supplied,
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
    fn into_request(self) -> Result<UpdateUserRequest, ContractError> {
        // the before-validator rejects a row whose selectors are both null
        if self.user_id.is_none() && self.user_email.is_none() {
            return Err(ContractError::from(USER_SELECTOR));
        }
        let fields = self.fields.into_fields();
        check_user_role(fields.user_role)?;
        Ok(UpdateUserRequest {
            fields,
            user_id: self.user_id,
            user_email: self.user_email,
        })
    }
}

// The wrapper is a typed wire record; each nested body keeps pydantic's absent and null states.
#[derive(Deserialize)]
struct BulkUpdateUserWire {
    #[serde(default)]
    users: Option<Vec<UpdateUserRequestWire>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    all_users: Option<Option<bool>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    user_updates: Option<Option<UpdateUserFieldsWire>>,
}

// GenerateRequestBase's max_budget validator runs before decoding a user record, so a blank string becomes null.
fn blank_row_budgets(fields: &mut Fields) {
    if let Some(JsonData::Array(rows)) = fields.get_mut("users") {
        for row in rows {
            if let JsonData::Object(row_fields) = row {
                blank_to_null(row_fields, &["max_budget"]);
            }
        }
    }
    if let Some(JsonData::Object(update_fields)) = fields.get_mut("user_updates") {
        blank_to_null(update_fields, &["max_budget"]);
    }
}

#[derive(Debug, Clone)]
pub struct BulkUpdateUserRequest {
    pub users: Option<Vec<UpdateUserRequest>>,
    pub all_users: Option<bool>,
    pub user_updates: Option<UpdateUserFields>,
}

fn decode_users(
    rows: Option<Vec<UpdateUserRequestWire>>,
) -> Result<Option<Vec<UpdateUserRequest>>, ContractError> {
    let Some(rows) = rows else {
        return Ok(None);
    };
    let mut requests = Vec::with_capacity(rows.len());
    for row in rows {
        requests.push(row.into_request()?);
    }
    Ok(Some(requests))
}

// UpdateUserRequestNoUserIDorEmail runs the same role validator without a selector check.
fn decode_updates(
    fields: Option<Option<UpdateUserFieldsWire>>,
) -> Result<Option<UpdateUserFields>, ContractError> {
    let Some(Some(update_fields)) = fields else {
        return Ok(None);
    };
    let decoded = update_fields.into_fields();
    check_user_role(decoded.user_role)?;
    Ok(Some(decoded))
}

impl BulkUpdateUserRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let mut fields = require_object(value)?;
        blank_row_budgets(&mut fields);
        let wire: BulkUpdateUserWire = decode(&fields)?;
        let supplied_updates = wire.user_updates.is_some();
        let request = Self {
            users: decode_users(wire.users)?,
            all_users: wire.all_users.unwrap_or(Some(false)),
            user_updates: decode_updates(wire.user_updates)?,
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

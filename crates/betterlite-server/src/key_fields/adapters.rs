use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_repository::config::ConfigParam;
use serde::Serialize;

use super::decode::to_object;
use super::error::DecodeError;

// The read-only view of a user row that /key/list expands keys with; the row's password never leaves it.
pub trait KeyListUserRow {
    fn user_id(&self) -> &str;

    fn user_email(&self) -> Option<&str>;

    fn user_alias(&self) -> Option<&str>;

    fn dumped(&self) -> Result<BTreeMap<String, JsonData>, DecodeError>;
}

impl KeyListUserRow for LiteLLMUserTable {
    fn user_id(&self) -> &str {
        &self.user_id
    }

    fn user_email(&self) -> Option<&str> {
        self.user_email.as_deref()
    }

    fn user_alias(&self) -> Option<&str> {
        self.user_alias.as_deref()
    }

    fn dumped(&self) -> Result<BTreeMap<String, JsonData>, DecodeError> {
        to_object(self)
    }
}

// A row still exposing only the pydantic v1 `dict()` shape dumps through its serde model.
pub trait LegacyDumpable: Serialize {
    fn legacy_dict(&self) -> Result<BTreeMap<String, JsonData>, DecodeError> {
        to_object(self)
    }
}

impl<T: Serialize> LegacyDumpable for T {}

// The environment-variables config row carries a mapping of names to values, which a caller reads typed.
#[must_use]
pub fn env_vars_param_value(param: &ConfigParam) -> Option<BTreeMap<String, String>> {
    let JsonData::Object(fields) = &param.param_value else {
        return None;
    };
    let mut variables = BTreeMap::new();
    for (name, value) in fields {
        let JsonData::String(value) = value else {
            return None;
        };
        variables.insert(name.clone(), value.clone());
    }
    Some(variables)
}

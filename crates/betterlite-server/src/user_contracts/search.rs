use betterlite_config::sections::settings::JsonValue;
use serde::{Deserialize, Serialize};

use crate::key_fields::values::InsensitiveContains;
use crate::proxy_request_contracts::{ContractError, decode, require_object};
use crate::user_contracts::wire::present_option;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSearchWhere {
    #[serde(rename = "OR")]
    pub or: Vec<UserSearchTerm>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserSearchTerm {
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub user_id: Option<InsensitiveContains>,
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub user_email: Option<InsensitiveContains>,
}

impl UserSearchWhere {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let where_clause: Self = decode(&fields)?;
        Ok(where_clause)
    }
}

use betterlite_config::sections::settings::JsonValue;
use serde::{Deserialize, Serialize};

use crate::management_envelopes::ResourceResponse;
use crate::proxy_request_contracts::{ContractError, decode, require_object};
use crate::user_contracts::wire::check_count;

pub const MAX_BULK_DELETE_USERS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkDeleteUserRequest {
    pub user_ids: Vec<String>,
}

impl BulkDeleteUserRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request: Self = decode(&fields)?;
        check_count("user_ids", request.user_ids.len(), MAX_BULK_DELETE_USERS)?;
        Ok(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDeleteResult {
    pub user_id: String,
    #[serde(default)]
    pub user_email: Option<String>,
    pub success: bool,
    #[serde(default)]
    pub teams_removed: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

pub type BulkDeleteUsersResponse = ResourceResponse<Vec<UserDeleteResult>>;

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_auth::vector_index_admin::lifecycle::is_index_lifecycle_request;
use betterlite_auth::vector_index_admin::{IndexAdminError, IndexOperation, require_index_admin};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Gate {
        user_role: Option<UserRole>,
        operation: IndexOperation,
    },
    Lifecycle {
        method: String,
        path: String,
        index_name: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::Gate {
                user_role,
                operation,
            } => gate(user_role.as_ref(), *operation),
            Input::Lifecycle {
                method,
                path,
                index_name,
            } => Ok(json!(is_index_lifecycle_request(method, path, index_name))),
        }
    }
}

fn gate(role: Option<&UserRole>, operation: IndexOperation) -> HelperResult {
    let mut fields = BTreeMap::new();
    if let Some(role) = role {
        fields.insert(
            String::from("user_role"),
            JsonData::String(role.to_string()),
        );
    }
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    let refusal = match require_index_admin(&auth, operation) {
        Ok(()) => None,
        Err(IndexAdminError::NotProxyAdmin(_)) => Some(403_u16),
    };
    Ok(json!({ "passed": refusal.is_none(), "status": refusal }))
}

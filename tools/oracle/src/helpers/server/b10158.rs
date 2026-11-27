use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::user_activity_scope::error::UserActivityScopeError;
// ast-grep-ignore: no-long-names
use betterlite_server::user_activity_scope::resolve_user_daily_activity_entity_ids;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    #[serde(default)]
    user_id: Option<String>,
    caller: Caller,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Caller {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_role: Option<UserRole>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            answers.push(answer(call)?);
        }
        Ok(serde_json::to_value(answers)?)
    }
}

fn answer(call: &Call) -> HelperResult {
    let auth = caller_key(&call.caller)?;
    // ast-grep-ignore: no-long-names
    let resolved = resolve_user_daily_activity_entity_ids(call.user_id.as_deref(), &auth);
    match resolved {
        Ok(entity_ids) => Ok(json!({"entity_ids": entity_ids})),
        Err(UserActivityScopeError::Denied(denied)) => Ok(json!({
            "denied": {
                "status_code": denied.status.status_code(),
                "reason": denied.reason,
            }
        })),
        Err(UserActivityScopeError::Caller(error)) => Ok(json!({
            "raised": {
                "status_code": error.status_code().map(|status| status.as_u16()),
                "detail": {"error": error.to_string()},
            }
        })),
    }
}

fn caller_key(caller: &Caller) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut fields = BTreeMap::new();
    if let Some(user_id) = &caller.user_id {
        fields.insert("user_id".to_owned(), JsonData::String(user_id.clone()));
    }
    if let Some(role) = caller.user_role {
        fields.insert("user_role".to_owned(), JsonData::String(role.to_string()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}

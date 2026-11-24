mod answers;
mod fakes;

use std::error::Error;

use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Caller {
    user_id: Option<String>,
    user_role: Option<UserRole>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selectors {
    user_id: Option<String>,
    team_id: Option<String>,
    organization_id: Option<String>,
    key_alias: Option<String>,
    key_hash: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Validate {
        caller: Caller,
        selectors: Selectors,
        user_row: Option<Value>,
        key_row: Option<Value>,
        key_fails: bool,
    },
    Teams {
        user: Option<Value>,
        teams: Option<Vec<Value>>,
        teams_fail: bool,
    },
    Projections {
        caller: Caller,
        teams: Vec<Value>,
    },
    AliasScope {
        caller: Caller,
        user_row: Option<Value>,
        query_params: Vec<JsonData>,
        where_parts: Vec<String>,
    },
}

#[derive(Debug)]
struct CallFailed;

// the oracle reports one failed outcome per call, so every error collapses into this marker
impl<E: Error> From<E> for CallFailed {
    fn from(_: E) -> Self {
        Self
    }
}

impl From<CallFailed> for HelperError {
    fn from(_: CallFailed) -> Self {
        Self {
            class: String::from("ValueError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: String::from("the key_list_filters case input does not decode"),
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let outcomes = input
            .calls
            .iter()
            .map(answers::answer)
            .collect::<Result<Vec<Value>, CallFailed>>()
            .map_err(HelperError::from)?;
        Ok(Value::Array(outcomes))
    }
}

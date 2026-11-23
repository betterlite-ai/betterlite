mod answers;
mod store;
mod support;

use std::error::Error;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_patch_fields::Row;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Metadata {
        request: Value,
        existing_metadata: Row,
    },
    BudgetCheck {
        field: BudgetField,
        value: Option<Decimal>,
    },
    SoftBudgetUpdate {
        request: Value,
        existing: Row,
        non_default_values: Row,
    },
    Token {
        request_key: Option<String>,
        existing_token: Option<String>,
    },
    Team {
        request_team: Option<String>,
        existing_team: Option<String>,
    },
    EndUser {
        request: Value,
    },
    Database,
    Prepare {
        request: Value,
        existing: Row,
    },
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum BudgetField {
    Soft,
    Max,
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
            message: String::from("the key_patch_fields case input does not decode"),
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

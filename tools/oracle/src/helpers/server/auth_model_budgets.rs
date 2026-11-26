#[path = "auth_model_budgets/cases.rs"]
mod cases;
#[path = "auth_model_budgets/doubles.rs"]
mod doubles;
#[path = "auth_model_budgets/end_user.rs"]
mod end_user;
#[path = "auth_model_budgets/spend.rs"]
mod spend;
#[path = "auth_model_budgets/support.rs"]
mod support;

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::cases::answer;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

// serde reads a missing Option field as None, so only the function has to be present
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    token: Option<BTreeMap<String, JsonData>>,
    models: Option<Vec<String>>,
    model_name: Option<String>,
    request_model: Option<String>,
    request_data: Option<BTreeMap<String, JsonData>>,
    path_params: Option<BTreeMap<String, JsonData>>,
    failing: Option<bool>,
    fallback: Option<String>,
    failing_models: Option<Vec<String>>,
    budget: Option<BTreeMap<String, JsonData>>,
    params: Option<BTreeMap<String, JsonData>>,
    end_user_id: Option<String>,
    max_budget: Option<f64>,
    spend: Option<f64>,
    database: Option<bool>,
    admin_name: Option<String>,
    tokens: Option<Vec<String>>,
    aliases: Option<BTreeMap<String, String>>,
    max_recurse_depth: Option<i64>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    KeyFallback,
    UserBudget,
    TeamBudget,
    EndUserApply,
    EndUserUpdate,
    GlobalSpend,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let answers: Vec<Value> = input
            .calls
            .iter()
            .map(|call| answer(call, &runtime))
            .collect();
        Ok(Value::Array(answers))
    }
}

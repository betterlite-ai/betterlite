use betterlite_auth::auth_context_sources::budgets::model_names_for_budget_checks;
use betterlite_auth::auth_request_models::RequestedModels;
use serde::Deserialize;
use serde_json::{Value, json};

use super::Input;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum BudgetModel {
    One(String),
    Many(Vec<String>),
}

pub(super) fn budget_answer(input: &Input) -> Value {
    let requested = match &input.model {
        None => RequestedModels::None,
        Some(BudgetModel::One(name)) => RequestedModels::One(name.clone()),
        Some(BudgetModel::Many(names)) => RequestedModels::Many(names.clone()),
    };
    json!(model_names_for_budget_checks(&requested))
}

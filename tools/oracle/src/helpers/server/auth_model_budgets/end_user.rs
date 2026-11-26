use std::collections::BTreeMap;

use betterlite_auth::auth_model_budgets::checks::ModelMaxBudget;
use betterlite_auth::auth_model_budgets::end_user::{
    EndUserParams, apply_budget_limits_to_end_user_params, update_valid_token_with_end_user_params,
};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::identity_rows::end_user::Region;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::Call;
use super::support::{Failure, budget_json, option_json, token_of};

// _apply_budget_limits_to_end_user_params: a nonnull row field wins even when it is zero or an empty map
pub(super) fn apply_value(call: &Call) -> Result<Value, Failure> {
    let fields = call
        .budget
        .clone()
        .ok_or(Failure::Missing("the budget table"))?;
    let value = serde_json::to_value(fields)?;
    let budget: LiteLLMBudgetTable = serde_json::from_value(value)?;
    let mut params = EndUserParams::default();
    apply_budget_limits_to_end_user_params(&mut params, &budget, call.end_user_id.as_deref());
    Ok(json!({
        "tpm": params.end_user_tpm_limit,
        "rpm": params.end_user_rpm_limit,
        "tpd": params.end_user_tpd_limit,
        "max_budget": option_json(params.end_user_max_budget),
        "model_max_budget": params.end_user_model_max_budget.as_ref().map_or(Value::Null, budget_json),
    }))
}

// update_valid_token_with_end_user_params: the id is always assigned, the other fields only when the params had them
pub(super) fn update_value(call: &Call) -> Result<Value, Failure> {
    let mut token = token_of(call)?;
    let params = params_of(call)?;
    update_valid_token_with_end_user_params(&mut token, &params);
    Ok(json!({
        "end_user_id": token.view.end_user_id.clone(),
        "tpm": token.view.end_user_tpm_limit,
        "rpm": token.view.end_user_rpm_limit,
        "tpd": token.view.end_user_tpd_limit,
        "max_budget": option_json(token.view.end_user_max_budget),
        "model_max_budget": token.view.end_user_model_max_budget.as_ref().map_or(Value::Null, |payload| budget_json(&payload.0)),
    }))
}

fn params_of(call: &Call) -> Result<EndUserParams, Failure> {
    let params = call
        .params
        .as_ref()
        .ok_or(Failure::Missing("the end user params"))?;
    Ok(EndUserParams {
        end_user_id: text_of(params, "end_user_id"),
        end_user_tpm_limit: int_of(params, "end_user_tpm_limit"),
        end_user_rpm_limit: int_of(params, "end_user_rpm_limit"),
        end_user_tpd_limit: int_of(params, "end_user_tpd_limit"),
        end_user_max_budget: amount_of(params, "end_user_max_budget"),
        end_user_model_max_budget: budget_of(params, "end_user_model_max_budget"),
        allowed_model_region: region_of(params, "allowed_model_region"),
    })
}

fn text_of(params: &BTreeMap<String, JsonData>, name: &str) -> Option<String> {
    match params.get(name) {
        Some(JsonData::String(value)) => Some(value.clone()),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        )
        | None => None,
    }
}

fn int_of(params: &BTreeMap<String, JsonData>, name: &str) -> Option<i64> {
    match params.get(name) {
        Some(JsonData::Number(number)) => number.as_i64(),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::String(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        )
        | None => None,
    }
}

fn amount_of(params: &BTreeMap<String, JsonData>, name: &str) -> Option<Decimal> {
    match params.get(name) {
        Some(JsonData::Number(number)) => number.to_string().parse().ok(),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::String(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        )
        | None => None,
    }
}

fn budget_of(params: &BTreeMap<String, JsonData>, name: &str) -> Option<ModelMaxBudget> {
    match params.get(name) {
        Some(JsonData::Object(map)) => Some(map.clone()),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_),
        )
        | None => None,
    }
}

fn region_of(params: &BTreeMap<String, JsonData>, name: &str) -> Option<Region> {
    let value = serde_json::to_value(params.get(name)?).ok()?;
    serde_json::from_value(value).ok()
}

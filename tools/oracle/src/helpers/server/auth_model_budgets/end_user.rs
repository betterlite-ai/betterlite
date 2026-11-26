use std::collections::BTreeMap;

use betterlite_auth::auth_model_budgets::end_user::{
    EndUserParams, apply_end_user_limits, update_end_user_token,
};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::Call;
use super::support::{Failure, decimal_json, token_of};

// _apply_budget_limits_to_end_user_params: a nonnull row field wins even when it is zero or an empty map
pub(super) fn apply_value(call: &Call) -> Result<Value, Failure> {
    let mut params = EndUserParams::default();
    let budget = budget_of(call)?;
    apply_end_user_limits(&mut params, &budget, call.end_user_id.as_deref());
    Ok(limits_json(&params))
}

fn budget_of(call: &Call) -> Result<LiteLLMBudgetTable, Failure> {
    let fields = call
        .budget
        .clone()
        .ok_or(Failure::Missing("the budget table"))?;
    Ok(serde_json::from_value(serde_json::to_value(fields)?)?)
}

// LiteLLM's update_valid_token_with_end_user_params: the id is always assigned, the other fields only when they came
pub(super) fn update_value(call: &Call) -> Result<Value, Failure> {
    let mut token = token_of(call)?;
    update_end_user_token(&mut token, &params_of(call)?);
    let params = EndUserParams {
        end_user_id: token.view.end_user_id.clone(),
        end_user_tpm_limit: token.view.end_user_tpm_limit,
        end_user_rpm_limit: token.view.end_user_rpm_limit,
        end_user_tpd_limit: token.view.end_user_tpd_limit,
        end_user_max_budget: token.view.end_user_max_budget,
        end_user_model_max_budget: token
            .view
            .end_user_model_max_budget
            .clone()
            .map(|payload| payload.0),
        allowed_model_region: token.extra.allowed_model_region.clone(),
    };
    Ok(limits_json(&params))
}

fn limits_json(params: &EndUserParams) -> Value {
    json!({
        "end_user_id": params.end_user_id,
        "allowed_model_region": params.allowed_model_region,
        "tpm": params.end_user_tpm_limit,
        "rpm": params.end_user_rpm_limit,
        "tpd": params.end_user_tpd_limit,
        "max_budget": params.end_user_max_budget.map_or(Value::Null, decimal_json),
        "model_max_budget": params.end_user_model_max_budget.as_ref().map_or(Value::Null, |budget| serde_json::to_value(budget).unwrap_or(Value::Null)),
    })
}

fn params_of(call: &Call) -> Result<EndUserParams, Failure> {
    let params = call
        .params
        .as_ref()
        .ok_or(Failure::Missing("the end user params"))?;
    Ok(EndUserParams {
        end_user_id: json_as(params, "end_user_id"),
        end_user_tpm_limit: json_as(params, "end_user_tpm_limit"),
        end_user_rpm_limit: json_as(params, "end_user_rpm_limit"),
        end_user_tpd_limit: json_as(params, "end_user_tpd_limit"),
        end_user_max_budget: json_as(params, "end_user_max_budget"),
        end_user_model_max_budget: json_as(params, "end_user_model_max_budget"),
        allowed_model_region: json_as(params, "allowed_model_region"),
    })
}

// the case sends the field as JSON her type declares it, so one round-trip covers string, count, amount, map and region
fn json_as<T: DeserializeOwned>(params: &BTreeMap<String, JsonData>, name: &str) -> Option<T> {
    serde_json::to_value(params.get(name)?)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
}

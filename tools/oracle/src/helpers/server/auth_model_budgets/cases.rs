use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_auth::auth_model_budgets::checks::{
    check_team_model_budget, check_user_model_budget,
};
use betterlite_auth::auth_model_budgets::fallback::{
    KeyBudgetError, check_key_model_budget_with_fallback,
};
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::database_reencryption::DEFAULT_RECURSE_DEPTH;
use betterlite_db::budget_permission_rows::JsonData;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::doubles::{CaseRewrite, ScriptedKeyLimiter, ScriptedTeamLimiter, ScriptedUserLimiter};
use super::end_user;
use super::spend;
use super::support::{Failure, budget_json, denial_json, error_value, token_of};
use super::{Call, Function};

pub(super) fn answer(call: &Call, runtime: &Runtime) -> Value {
    match execute(call, runtime) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(_failure) => json!({"ok": false}),
    }
}

fn execute(call: &Call, runtime: &Runtime) -> Result<Value, Failure> {
    match call.function {
        Function::KeyFallback => runtime.block_on(key_fallback_value(call)),
        Function::UserBudget => runtime.block_on(user_budget_value(call)),
        Function::TeamBudget => runtime.block_on(team_budget_value(call)),
        Function::EndUserApply => end_user::apply_value(call),
        Function::EndUserUpdate => end_user::update_value(call),
        Function::GlobalSpend => runtime.block_on(spend::global_spend_value(call)),
    }
}

// _check_key_model_budget_with_fallback: the model moves only when the fallback is authorized, and only a budget error records one
async fn key_fallback_value(call: &Call) -> Result<Value, Failure> {
    let token = token_of(call)?;
    let model_name = call
        .model_name
        .as_deref()
        .ok_or(Failure::Missing("the model name"))?;
    let limiter = ScriptedKeyLimiter {
        failing: call.failing.unwrap_or(false),
        fallback: call.fallback.clone(),
        calls: Mutex::new(Vec::new()),
    };
    let mut request = request_of(call)?;
    let error = fallback_error(&limiter, &token, model_name, &mut request, call).await?;
    Ok(json!({
        "calls": limiter.calls(),
        "request_model": model_of(request.request_data.as_ref()),
        "parsed_model": model_of(request.parsed_body.as_ref()),
        "json_model": model_of(request.json_cache.as_ref()),
        "body_model": body_model(request.serialized_body.as_deref())?,
        "path_model": model_of(request.path_params.as_ref()),
        "error": error,
    }))
}

fn request_of(call: &Call) -> Result<CaseRewrite, Failure> {
    let request_data = call
        .request_data
        .clone()
        .ok_or(Failure::Missing("the request data"))?;
    Ok(CaseRewrite {
        request_data: Some(request_data),
        parsed_body: None,
        json_cache: None,
        serialized_body: None,
        path_params: call.path_params.clone(),
    })
}

async fn fallback_error(
    limiter: &ScriptedKeyLimiter,
    token: &UserAPIKeyAuth,
    model_name: &str,
    request: &mut CaseRewrite,
    call: &Call,
) -> Result<Value, Failure> {
    let aliases = call.aliases.clone().unwrap_or_default();
    let provider = UnresolvedProviderInference;
    let context = ModelAccessContext {
        view: None,
        global_model_aliases: &aliases,
        compaction: None,
        provider: &provider,
        max_recurse_depth: call.max_recurse_depth.unwrap_or(DEFAULT_RECURSE_DEPTH),
    };
    match check_key_model_budget_with_fallback(
        limiter,
        token,
        model_name,
        call.request_model.as_deref(),
        request,
        &context,
        None,
    )
    .await
    {
        Ok(()) => Ok(Value::Null),
        Err(KeyBudgetError::Budget(details)) => Ok(denial_json(&details)),
        Err(_other) => Err(Failure::Limiter),
    }
}

// _check_user_model_budget: every model in order and the limiter's bool is not a verdict
async fn user_budget_value(call: &Call) -> Result<Value, Failure> {
    let token = token_of(call)?;
    let models = call.models.clone().ok_or(Failure::Missing("the models"))?;
    let limiter = ScriptedUserLimiter {
        failing_models: call.failing_models.clone().unwrap_or_default(),
        seen: Mutex::new(Vec::new()),
    };
    let outcome = check_user_model_budget(&limiter, &token, &models).await;
    let calls: Vec<Value> = limiter
        .seen()
        .into_iter()
        .map(|seen| {
            json!({
                "user_id": seen.user_id,
                "model": seen.model,
                "budget": budget_json(&seen.budget),
            })
        })
        .collect();
    Ok(json!({"calls": calls, "error": error_value(outcome)?}))
}

// _check_team_model_budget: the key's own map travels unchanged so the limiter can honour its overrides
async fn team_budget_value(call: &Call) -> Result<Value, Failure> {
    let token = token_of(call)?;
    let models = call.models.clone().ok_or(Failure::Missing("the models"))?;
    let limiter = ScriptedTeamLimiter {
        failing_models: call.failing_models.clone().unwrap_or_default(),
        seen: Mutex::new(Vec::new()),
    };
    let outcome = check_team_model_budget(&limiter, &token, &models).await;
    let calls: Vec<Value> = limiter
        .seen()
        .into_iter()
        .map(|seen| {
            json!({
                "team_id": seen.team_id,
                "model": seen.model,
                "team_budget": budget_json(&seen.team_budget),
                "key_budget": seen.key_budget.as_ref().map_or(Value::Null, budget_json),
            })
        })
        .collect();
    Ok(json!({"calls": calls, "error": error_value(outcome)?}))
}

fn model_of(data: Option<&BTreeMap<String, JsonData>>) -> Value {
    let Some(model) = data.and_then(|fields| fields.get("model")) else {
        return Value::Null;
    };
    serde_json::to_value(model).unwrap_or(Value::Null)
}

fn body_model(body: Option<&[u8]>) -> Result<Value, Failure> {
    let Some(bytes) = body else {
        return Ok(Value::Null);
    };
    let data: BTreeMap<String, JsonData> = serde_json::from_slice(bytes)?;
    Ok(model_of(Some(&data)))
}

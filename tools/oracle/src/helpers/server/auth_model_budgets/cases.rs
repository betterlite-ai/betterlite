use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_auth::auth_model_budgets::checks::{
    check_team_model_budget, check_user_model_budget,
};
use betterlite_auth::auth_model_budgets::fallback::{
    KeyBudgetCall, KeyBudgetError, check_key_budget_fallback,
};
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::database_reencryption::DEFAULT_RECURSE_DEPTH;
use betterlite_db::budget_permission_rows::JsonData;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::doubles::{
    CaseRewrite, ScriptedKeyLimiter, ScriptedTeamLimiter, ScriptedUserLimiter, SeenUser,
};
use super::end_user;
use super::spend;
use super::support::{Failure, denial_json, error_value, token_of};
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
        Function::UserBudget => runtime.block_on(budget_value(call, BudgetEntity::User)),
        Function::TeamBudget => runtime.block_on(budget_value(call, BudgetEntity::Team)),
        Function::EndUserApply => end_user::apply_value(call),
        Function::EndUserUpdate => end_user::update_value(call),
        Function::GlobalSpend => runtime.block_on(spend::global_spend_value(call)),
    }
}

enum BudgetEntity {
    User,
    Team,
}

// _check_user_model_budget and _check_team_model_budget: every model in order and the limiter's bool is not a verdict
async fn budget_value(call: &Call, entity: BudgetEntity) -> Result<Value, Failure> {
    let token = token_of(call)?;
    let models = call.models.clone().ok_or(Failure::Missing("the models"))?;
    match entity {
        BudgetEntity::User => {
            let limiter = ScriptedUserLimiter {
                failing_models: call.failing_models.clone().unwrap_or_default(),
                seen: Mutex::new(Vec::new()),
            };
            let outcome = check_user_model_budget(&limiter, &token, &models).await;
            let calls: Vec<Value> = limiter.seen().into_iter().map(seen_json).collect();
            Ok(json!({"calls": calls, "error": error_value(outcome)?}))
        }
        BudgetEntity::Team => {
            let limiter = ScriptedTeamLimiter {
                failing_models: call.failing_models.clone().unwrap_or_default(),
                seen: Mutex::new(Vec::new()),
            };
            let outcome = check_team_model_budget(&limiter, &token, &models).await;
            Ok(json!({"calls": limiter.seen(), "error": error_value(outcome)?}))
        }
    }
}

fn seen_json(seen: SeenUser) -> Value {
    json!({
        "user_id": seen.user_id,
        "model": seen.model,
        "budget": serde_json::to_value(&seen.budget).unwrap_or(Value::Null),
    })
}

// _check_key_model_budget_with_fallback: the model moves only when the fallback is authorized, and only a budget error records one
async fn key_fallback_value(call: &Call) -> Result<Value, Failure> {
    let limiter = key_limiter_of(call);
    let mut request = request_of(call)?;
    let model_name = call
        .model_name
        .as_deref()
        .ok_or(Failure::Missing("the model name"))?;
    let error = fallback_error(&limiter, &token_of(call)?, model_name, &mut request, call).await?;
    key_answer(&request, limiter.calls(), error)
}

fn key_limiter_of(call: &Call) -> ScriptedKeyLimiter {
    ScriptedKeyLimiter {
        failing: call.failing.unwrap_or(false),
        fallback: call.fallback.clone(),
        calls: Mutex::new(Vec::new()),
    }
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

fn key_answer(request: &CaseRewrite, calls: Vec<String>, error: Value) -> Result<Value, Failure> {
    Ok(json!({
        "calls": calls,
        "request_model": model_of(request.request_data.as_ref()),
        "parsed_model": model_of(request.parsed_body.as_ref()),
        "json_model": model_of(request.json_cache.as_ref()),
        "body_model": body_model(request.serialized_body.as_deref())?,
        "path_model": model_of(request.path_params.as_ref()),
        "error": error,
    }))
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
    let callee = KeyBudgetCall {
        limiter,
        token,
        model_name,
        request_model: call.request_model.as_deref(),
    };
    match check_key_budget_fallback(callee, request, &context, None).await {
        Ok(()) => Ok(Value::Null),
        Err(KeyBudgetError::Budget(details)) => Ok(denial_json(&details)),
        Err(_other) => Err(Failure::Limiter),
    }
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

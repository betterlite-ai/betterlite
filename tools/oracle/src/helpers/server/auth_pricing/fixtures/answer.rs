use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_key_budgets::key_budget::Denial;
use betterlite_auth::auth_key_budgets::money::Money;
use betterlite_auth::auth_pricing::catalog::PricingError;
use betterlite_auth::auth_pricing::project::{ProjectError, run_project_checks};
use betterlite_auth::auth_pricing::request::{
    RequestFacts, enforce_user_param_check, global_proxy_budget_check,
    reject_clientside_metadata_tags_check,
};
use betterlite_auth::auth_pricing::routes::{
    request_skips_budget_checks, route_skips_budget_checks,
};
use betterlite_auth::auth_pricing::unpriced::{
    unpriced_models_block_message, unpriced_models_in_request,
};
use betterlite_auth::auth_pricing::zero_cost::is_model_cost_zero;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::auth_tenant_budgets::TenantAdmission;
use betterlite_auth::auth_tenant_budgets::error::TenantBudgetError;
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_error::proxy_exception::ProxyException;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::collaborators::{NoGrant, ProjectAlerts, ScriptedSpend, denial_value, envelope};
use super::input::{Call, Function};
use super::router::ScriptedCatalog;

// the production call for this function, both sides recording the same facts around it
pub(crate) async fn answer(call: &Call) -> Result<Value, Failure> {
    let model = call.requested();
    let catalog = ScriptedCatalog::new(&call.router);
    let mut answer = match call.function {
        Function::ZeroCost => zero_cost(&catalog, &model),
        Function::Unpriced => unpriced(&catalog, &model)?,
        Function::RequestSkip => request_skip(call, &catalog, &model)?,
        Function::RequestChecks => request_checks(call, &catalog, &model)?,
        Function::Project => project(call, &catalog, &model).await?,
    };
    if let Some(fields) = answer.as_object_mut() {
        fields.insert("cache".to_owned(), catalog.cache_value());
        fields.insert("lookups".to_owned(), json!(catalog.lookups()));
    }
    Ok(answer)
}

fn zero_cost(catalog: &ScriptedCatalog, model: &RequestedModels) -> Value {
    json!({
        "zero": is_model_cost_zero(model, Some(catalog)),
    })
}

fn unpriced(catalog: &ScriptedCatalog, model: &RequestedModels) -> Result<Value, Failure> {
    let models = unpriced_models_in_request(model, Some(catalog))?;
    Ok(json!({
        "unpriced": models,
        "message": unpriced_models_block_message(&models),
    }))
}

fn request_skip(
    call: &Call,
    catalog: &ScriptedCatalog,
    model: &RequestedModels,
) -> Result<Value, Failure> {
    let Some(route) = call.route.as_deref() else {
        return Err(Failure::MissingRoute);
    };
    Ok(json!({
        "route_skips": route_skips_budget_checks(route),
        "request_skips": request_skips_budget_checks(route, model, Some(catalog)),
    }))
}

// the three request-level checks run as the stage runs them, each with its own public envelope
fn request_checks(
    call: &Call,
    catalog: &ScriptedCatalog,
    model: &RequestedModels,
) -> Result<Value, Failure> {
    let Some(route) = call.route.as_deref() else {
        return Err(Failure::MissingRoute);
    };
    let facts = RequestFacts {
        method: call.method.as_deref(),
        route,
        body: &call.body,
    };
    let skip = request_skips_budget_checks(route, model, Some(catalog));
    let tags = reject_clientside_metadata_tags_check(call.reject, &facts)
        .err()
        .map(|exception| envelope(&exception));
    let user = enforce_user_param_check(call.enforce, &facts)
        .err()
        .map(|error| user_envelope(&error.to_string()));
    let budget = global_proxy_budget_check(
        budget_money(call.max_budget),
        global_spend(call.global_spend),
        skip,
        route,
    )
    .map(|denied| {
        json!({
            "denial": denial_value(&denied),
            "envelope": envelope(&budget_exception(&denied)),
        })
    });
    Ok(json!({
        "tags": tags,
        "user": user,
        "budget": budget,
        "skip": skip,
    }))
}

async fn project(
    call: &Call,
    catalog: &ScriptedCatalog,
    model: &RequestedModels,
) -> Result<Value, Failure> {
    let Some(route) = call.route.as_deref() else {
        return Err(Failure::MissingRoute);
    };
    let skip = request_skips_budget_checks(route, model, Some(catalog));
    let reader = ScriptedSpend::new(&call.spends);
    let alerts = ProjectAlerts::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&alerts),
    };
    let token = token_of(call)?;
    let outcome = run_project_checks(
        call.project.as_ref(),
        model,
        skip,
        token.as_ref(),
        &NoGrant,
        &admission,
    )
    .await;
    let mut record = project_record(outcome)?;
    if let Some(fields) = record.as_object_mut() {
        fields.insert("reads".to_owned(), Value::Array(reader.reads()));
        fields.insert("alerts".to_owned(), Value::Array(alerts.alerts()));
    }
    Ok(record)
}

fn project_record(outcome: Result<(), ProjectError>) -> Result<Value, Failure> {
    match outcome {
        Ok(()) => Ok(json!({
            "class": "ok",
            "denial": Value::Null,
            "message": Value::Null,
            "envelope": Value::Null,
        })),
        Err(error @ ProjectError::Blocked { .. }) => {
            let message = error.to_string();
            let failure = AuthFailure::Other {
                message: message.clone(),
                param: None,
            };
            let client = as_proxy_exception(&failure);
            Ok(json!({
                "class": "blocked",
                "denial": Value::Null,
                "message": message,
                "envelope": envelope(client.exception()),
            }))
        }
        Err(ProjectError::Budget(TenantBudgetError::Denied(denied))) => Ok(json!({
            "class": "denied",
            "denial": denial_value(&denied),
            "message": Value::Null,
            "envelope": envelope(&budget_exception(&denied)),
        })),
        Err(
            error @ (ProjectError::Grant(_)
            | ProjectError::Budget(TenantBudgetError::Spend(_) | TenantBudgetError::Tag(_))),
        ) => Err(Failure::Project(error)),
    }
}

fn token_of(call: &Call) -> Result<Option<UserAPIKeyAuth>, AuthError> {
    match &call.token {
        Some(fields) => Ok(Some(UserAPIKeyAuth::construct(AuthInput::Dict(
            fields.clone(),
        ))?)),
        None => Ok(None),
    }
}

// an ordinary exception reaches the client as B757's 401 auth_error
fn user_envelope(message: &str) -> Value {
    let failure = AuthFailure::Other {
        message: message.to_owned(),
        param: None,
    };
    let client = as_proxy_exception(&failure);
    envelope(client.exception())
}

// B757's budget_exceeded envelope: 422 with no param
fn budget_exception(denial: &Denial) -> ProxyException {
    let failure = AuthFailure::BudgetExceeded {
        message: denial.message.clone(),
        status_code: Some(denial.status_code),
        llm_provider: String::new(),
    };
    let client = as_proxy_exception(&failure);
    client.exception().clone()
}

fn budget_money(budget: Option<f64>) -> Money {
    match budget {
        Some(value) => Money::from_f64(value),
        None => Money::Finite(Decimal::ZERO),
    }
}

fn global_spend(spend: Option<f64>) -> Option<Decimal> {
    match spend {
        Some(value) => Money::from_f64(value).finite_value(),
        None => None,
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Pricing(#[from] PricingError),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("the call needs a route")]
    MissingRoute,
}

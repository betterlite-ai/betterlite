mod oracle_cache;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::model_budget_limiter::ModelBudgetLimiter;
use betterlite_auth::model_budget_limiter::charging::{
    ChargeMetadata, PayloadIdentity, SuccessEvent, log_success_event,
};
use betterlite_auth::model_budget_limiter::clock::BudgetClock;
use betterlite_auth::model_budget_limiter::enforcement::end_user_within_model_budget;
use betterlite_auth::model_budget_limiter::enforcement::{
    key_within_model_budget, team_within_model_budget, user_within_model_budget,
};
use betterlite_auth::model_budget_limiter::fallbacks::{
    FallbackQuery, fallback_model_within_budget,
};
use betterlite_auth::model_budget_values::amount_text::amount_from_text;
use betterlite_auth::model_budget_values::candidates::BedrockCatalog;
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

use oracle_cache::OracleCache;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    uptime_seconds: f64,
    #[serde(default)]
    memory: BTreeMap<String, JsonData>,
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Key(CheckCall),
    User(CheckCall),
    EndUser(CheckCall),
    Team(TeamCall),
    Fallback(FallbackCall),
    Charge(Box<ChargeCall>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckCall {
    entity_id: String,
    #[serde(default)]
    key_alias: Option<String>,
    model: String,
    #[serde(default)]
    budget: Option<BTreeMap<String, JsonData>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamCall {
    team_id: String,
    model: String,
    #[serde(default)]
    budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    key_budget: Option<BTreeMap<String, JsonData>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FallbackCall {
    entity_id: String,
    #[serde(default)]
    key_alias: Option<String>,
    model: String,
    #[serde(default)]
    budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    fallbacks: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChargeCall {
    #[serde(default)]
    model_group: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    response_cost: Option<serde_json::Number>,
    #[serde(default)]
    key_budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    team_budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    user_budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    end_user_budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    key_hash: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    end_user: Option<String>,
    #[serde(default)]
    end_user_id: Option<String>,
    #[serde(default)]
    call_type: Option<String>,
    #[serde(default)]
    batch_id: Option<String>,
}

struct CoreDurations(TimeZoneDatabase);

impl DurationParser for CoreDurations {
    fn seconds(&self, spec: &str, now: DateTime<Utc>) -> Option<i64> {
        duration_in_seconds(spec, now, None, &self.0)
            .ok()
            .map(|duration| duration.seconds)
    }
}

struct NoBedrock;

impl BedrockCatalog for NoBedrock {
    fn base_model(&self, model: &str) -> String {
        model.to_owned()
    }

    fn prices_as_bedrock(&self, _base_model: &str) -> bool {
        false
    }
}

struct FixedClock {
    uptime: Duration,
}

impl BudgetClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }

    fn uptime(&self) -> Duration {
        self.uptime
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(failure)?;
        runtime.block_on(execute(input))
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

async fn execute(input: &Input) -> HelperResult {
    let uptime = Duration::try_from_secs_f64(input.uptime_seconds)
        .map_err(|_source| oracle_cache::invalid("uptime_seconds is not a duration".to_owned()))?;
    let cache = OracleCache::new(&input.memory)?;
    let limiter = ModelBudgetLimiter {
        cache,
        durations: Box::new(CoreDurations(TimeZoneDatabase::from_env())),
        catalog: Box::new(NoBedrock),
        clock: Box::new(FixedClock { uptime }),
        budget_exceeded_status: 422,
    };
    let mut outcomes = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        outcomes.push(run_call(&limiter, call).await);
    }
    let (spend, markers) = limiter.cache.state();
    Ok(json!({
        "calls": outcomes,
        "spend": spend,
        "markers": markers,
    }))
}

async fn run_call(limiter: &ModelBudgetLimiter<OracleCache>, call: &Call) -> Value {
    match call {
        Call::Key(check) => key_call(limiter, check).await,
        Call::User(check) => user_call(limiter, check).await,
        Call::EndUser(check) => end_user_call(limiter, check).await,
        Call::Team(check) => team_call(limiter, check).await,
        Call::Fallback(check) => fallback_outcome(limiter, check).await,
        Call::Charge(charge) => charge_outcome(limiter, charge).await,
    }
}

async fn key_call(limiter: &ModelBudgetLimiter<OracleCache>, check: &CheckCall) -> Value {
    let result = key_within_model_budget(
        limiter,
        &check.entity_id,
        check.key_alias.as_deref(),
        check.budget.as_ref(),
        &check.model,
    )
    .await;
    check_outcome(result)
}

async fn user_call(limiter: &ModelBudgetLimiter<OracleCache>, check: &CheckCall) -> Value {
    let result = user_within_model_budget(
        limiter,
        &check.entity_id,
        check.budget.as_ref(),
        &check.model,
    )
    .await;
    check_outcome(result)
}

async fn end_user_call(limiter: &ModelBudgetLimiter<OracleCache>, check: &CheckCall) -> Value {
    let result = end_user_within_model_budget(
        limiter,
        &check.entity_id,
        check.budget.as_ref(),
        &check.model,
    )
    .await;
    check_outcome(result)
}

async fn team_call(limiter: &ModelBudgetLimiter<OracleCache>, check: &TeamCall) -> Value {
    let result = team_within_model_budget(
        limiter,
        &check.team_id,
        check.budget.as_ref(),
        check.key_budget.as_ref(),
        &check.model,
    )
    .await;
    check_outcome(result)
}

fn check_outcome(result: Result<(), ErrorClass>) -> Value {
    let Err(error) = result else {
        return json!({"outcome": "within"});
    };
    if let ErrorClass::BudgetExceeded(details) = &error {
        return exceeded_outcome(details);
    }
    json!({"outcome": "error"})
}

fn exceeded_outcome(details: &BudgetExceededDetails) -> Value {
    json!({
        "outcome": "exceeded",
        "current_cost": details.current_cost.as_f64(),
        "max_budget": details.max_budget.as_f64(),
        "entity_type": details.entity_type,
        "entity_id": details.entity_id,
    })
}

async fn fallback_outcome(
    limiter: &ModelBudgetLimiter<OracleCache>,
    check: &FallbackCall,
) -> Value {
    let result = fallback_model_within_budget(
        limiter,
        FallbackQuery {
            entity_id: &check.entity_id,
            key_alias: check.key_alias.as_deref(),
            model_max_budget: check.budget.as_ref(),
            budget_fallbacks: check.fallbacks.as_ref(),
            model: &check.model,
        },
    )
    .await;
    let Ok(chosen) = result else {
        return json!({"outcome": "error"});
    };
    json!({"outcome": "fallback", "model": chosen})
}

fn success_event(charge: &ChargeCall, response_cost: Decimal) -> SuccessEvent<'_> {
    SuccessEvent {
        model_group: charge.model_group.as_deref(),
        model: charge.model.as_deref(),
        response_cost,
        metadata: ChargeMetadata {
            key_max_budget: charge.key_budget.as_ref(),
            team_max_budget: charge.team_budget.as_ref(),
            user_max_budget: charge.user_budget.as_ref(),
            end_user_max_budget: charge.end_user_budget.as_ref(),
        },
        payload: PayloadIdentity {
            key_hash: charge.key_hash.as_deref(),
            team_id: charge.team_id.as_deref(),
            user_id: charge.user_id.as_deref(),
            end_user: charge.end_user.as_deref(),
            end_user_id: charge.end_user_id.as_deref(),
        },
        call_type: charge.call_type.as_deref(),
        batch_id: charge.batch_id.as_deref(),
    }
}

async fn charge_outcome(limiter: &ModelBudgetLimiter<OracleCache>, charge: &ChargeCall) -> Value {
    let parsed = charge
        .response_cost
        .as_ref()
        .map_or(Some(Decimal::ZERO), |number| {
            amount_from_text(&number.to_string())
        });
    let Some(cost) = parsed else {
        return json!({"outcome": "raised"});
    };
    let event = success_event(charge, cost);
    if let Err(_error) = log_success_event(limiter, Some(&event)).await {
        return json!({"outcome": "raised"});
    }
    json!({"outcome": "charged"})
}

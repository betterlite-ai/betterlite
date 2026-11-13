pub(crate) mod input;
mod oracle_cache;

use std::time::Duration;

use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::model_budget_limiter::ModelBudgetLimiter;
use betterlite_auth::model_budget_limiter::charging::{
    ChargeMetadata, PayloadIdentity, SuccessEvent, log_success_event,
};
use betterlite_auth::model_budget_limiter::clock::BudgetClock;
use betterlite_auth::model_budget_limiter::enforcement::{
    BudgetSubject, subject_within_model_budget,
};
use betterlite_auth::model_budget_limiter::fallbacks::{
    FallbackQuery, fallback_model_within_budget,
};
use betterlite_auth::model_budget_values::amount_text::amount_from_text;
use betterlite_auth::model_budget_values::candidates::BedrockCatalog;
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

use input::{Call, ChargeCall, FallbackCall, Input, TeamCall};
use oracle_cache::OracleCache;

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
        Call::Key(check) => {
            let subject = BudgetSubject::Key {
                entity_id: &check.entity_id,
                key_alias: check.key_alias.as_deref(),
            };
            check_outcome(
                subject_within_model_budget(limiter, &subject, check.budget.as_ref(), &check.model)
                    .await,
            )
        }
        Call::User(check) => {
            let subject = BudgetSubject::User {
                user_id: &check.entity_id,
            };
            check_outcome(
                subject_within_model_budget(limiter, &subject, check.budget.as_ref(), &check.model)
                    .await,
            )
        }
        Call::EndUser(check) => {
            let subject = BudgetSubject::EndUser {
                end_user_id: &check.entity_id,
            };
            check_outcome(
                subject_within_model_budget(limiter, &subject, check.budget.as_ref(), &check.model)
                    .await,
            )
        }
        Call::Team(check) => team_call(limiter, check).await,
        Call::Fallback(check) => fallback_outcome(limiter, check).await,
        Call::Charge(charge) => charge_outcome(limiter, charge).await,
    }
}

async fn team_call(limiter: &ModelBudgetLimiter<OracleCache>, check: &TeamCall) -> Value {
    check_outcome(
        subject_within_model_budget(
            limiter,
            &BudgetSubject::Team {
                team_id: &check.team_id,
                key_model_max_budget: check.key_budget.as_ref(),
            },
            check.budget.as_ref(),
            &check.model,
        )
        .await,
    )
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
    let query = FallbackQuery {
        entity_id: &check.entity_id,
        key_alias: check.key_alias.as_deref(),
        model_max_budget: check.budget.as_ref(),
        budget_fallbacks: check.fallbacks.as_ref(),
        model: &check.model,
    };
    let chosen = fallback_model_within_budget(limiter, query).await;
    json!({
        "outcome": "fallback",
        "model": chosen,
    })
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

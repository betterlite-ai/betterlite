mod fixtures;

use std::collections::BTreeMap;

use betterlite_auth::budget_counter::BudgetCounter;
use betterlite_auth::model_budget_values::amount_text::amount_from_text;
use betterlite_auth::reservation_admission::engine::{Admission, AdmissionRequest, admit};
use betterlite_auth::reservation_admission::error::AdmissionError;
use betterlite_auth::reservation_counters::{CounterInput, CounterServices, budget_counters};
use betterlite_auth::reservation_settlement::entry::ReservationEntry;
use betterlite_auth::reservation_settlement::release::release_entries_best_effort;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redis::request_batch::lock;
use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::fixtures::fakes::ScriptedMemory;
use self::fixtures::input::{Input, User};
use self::fixtures::services::{Durations, NoCache, NoGroups, NoTags};

// litellm.budget_exceeded_status_code's default, the status every BudgetExceededError carries
const BUDGET_EXCEEDED_STATUS: u16 = 422;
// the counter builder needs a clock its window start can subtract a day from
const NOW_SECONDS: i64 = 1_704_067_200;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|error| invalid(error.to_string()))?;
        runtime.block_on(execute(input))
    }
}

// reserve_budget_for_request's flow: build the counters, initialize and charge them, and release every applied entry when the reservation is rejected before the error goes back out
async fn execute(input: &Input) -> HelperResult {
    let token = input.token.auth()?;
    let user = input.user.as_ref().map(User::row).transpose()?;
    let counters = admission_counters(&token, user.as_ref()).await?;
    let memory = ScriptedMemory::new(
        &input.memory,
        &input.sources,
        &input.failures,
        input.pipeline_fails,
    )?;
    let Some(reservation_cost) = amount_from_text(&input.reservation_cost.to_string()) else {
        return Err(invalid(format!(
            "reservation_cost {} is not a decimal",
            input.reservation_cost
        )));
    };
    let request = AdmissionRequest {
        counters: &counters,
        token: Some(&token),
        reservation_cost,
        fail_closed_budget_enforcement: input.fail_closed,
        budget_exceeded_status: BUDGET_EXCEEDED_STATUS,
        throttle_percentage: input.throttle_percentage,
    };
    let mut applied = Vec::new();
    let outcome = admit(&memory, &request, &mut applied).await;
    let (reservation, error) =
        outcome_values(&outcome, &mut applied, &memory, reservation_cost).await?;
    let values = lock(&memory.values);
    let pipelines = lock(&memory.pipelines);
    Ok(json!({
        "reservation": reservation,
        "error": error,
        "memory": memory_value(&values)?,
        "pipelines": pipelines_value(&pipelines)?,
        "invalidated": &*lock(&memory.invalidated),
    }))
}

// the reservation the caller settles against, or the error after every applied entry is released
async fn outcome_values(
    outcome: &Result<Admission, AdmissionError>,
    applied: &mut Vec<ReservationEntry>,
    memory: &ScriptedMemory,
    reservation_cost: Decimal,
) -> Result<(Value, Value), Box<HelperError>> {
    match outcome {
        Ok(admission) if !applied.is_empty() => Ok((
            reservation_value(admission.reserved_cost, applied)?,
            Value::Null,
        )),
        Ok(_) => Ok((Value::Null, Value::Null)),
        Err(failure) => {
            release_entries_best_effort(memory, applied, reservation_cost).await;
            Ok((Value::Null, error_value(failure)))
        }
    }
}

// _get_budget_counters: the counters this token and its user row give the reservation
async fn admission_counters(
    token: &UserAPIKeyAuth,
    user: Option<&LiteLLMUserTable>,
) -> Result<Vec<BudgetCounter>, Box<HelperError>> {
    let Some(now) = DateTime::from_timestamp(NOW_SECONDS, 0) else {
        return Err(invalid(
            "the fixed clock is outside the supported range".to_owned(),
        ));
    };
    let cache = NoCache;
    let tags = NoTags;
    let groups = NoGroups;
    let durations = Durations(TimeZoneDatabase::from_env());
    let input = CounterInput {
        token,
        team: None,
        user,
        end_user_id: None,
        end_user: None,
        tag_names: &[],
        apply_user_budget_to_team_keys: false,
    };
    let services = CounterServices {
        database: false,
        cache: &cache,
        tags: &tags,
        groups: &groups,
        durations: &durations,
        now,
    };
    budget_counters(&input, &services)
        .await
        .map_err(|error| invalid(error.to_string()))
}

fn reservation_value(
    reserved_cost: Decimal,
    applied: &[ReservationEntry],
) -> Result<Value, Box<HelperError>> {
    let mut entries = Vec::new();
    for entry in applied {
        let Some(counter_key) = &entry.counter_key else {
            continue;
        };
        let Some(entry_cost) = entry.reserved_cost else {
            continue;
        };
        entries.push(json!({
            "counter_key": counter_key,
            "reserved_cost": amount_value(entry_cost)?,
            "applied_adjustment": amount_value(entry.applied_adjustment)?,
        }));
    }
    Ok(json!({
        "reserved_cost": amount_value(reserved_cost)?,
        "entries": entries,
    }))
}

fn memory_value(values: &BTreeMap<String, Decimal>) -> Result<Value, Box<HelperError>> {
    let mut amounts = BTreeMap::new();
    for (key, value) in values {
        let Some(float) = value.to_f64() else {
            return Err(invalid(format!(
                "the spend counter {key} has no float form"
            )));
        };
        let _previous = amounts.insert(key.clone(), float);
    }
    Ok(json!(amounts))
}

fn amount_value(value: Decimal) -> Result<Value, Box<HelperError>> {
    let Some(float) = value.to_f64() else {
        return Err(invalid(format!("the amount {value} has no float form")));
    };
    let Some(number) = serde_json::Number::from_f64(float) else {
        return Err(invalid(format!("the amount {value} is not a finite float")));
    };
    Ok(Value::Number(number))
}

fn pipelines_value(pipelines: &[Vec<(String, Decimal)>]) -> Result<Value, Box<HelperError>> {
    let mut calls = Vec::new();
    for pending in pipelines {
        let mut recorded = Vec::new();
        for (counter_key, increment) in pending {
            let Some(float) = increment.to_f64() else {
                return Err(invalid(format!(
                    "the pipeline increment for {counter_key} has no float form"
                )));
            };
            recorded.push(json!([counter_key, float]));
        }
        calls.push(Value::Array(recorded));
    }
    Ok(Value::Array(calls))
}

fn error_value(error: &AdmissionError) -> Value {
    match error {
        AdmissionError::BudgetExceeded { details } => json!({
            "class": "BudgetExceededError",
            "status": details.status_code,
        }),
        AdmissionError::Unavailable { .. } => json!({
            "class": "HTTPException",
            "status": 503,
        }),
        AdmissionError::Overflow | AdmissionError::Settlement(_) => json!({
            "class": "AdmissionError",
            "status": 500,
        }),
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_db::spend_queues::daily::{DailySpendUpdate, aggregate_daily_rows};
use betterlite_db::spend_queues::entity::{SpendUpdateQueueItem, aggregate_db_transactions};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: BTreeMap<String, Step>,
}

// an internally tagged enum buffers its payload where the daily row's RawValue cannot be read, so dispatch here
#[derive(Debug, Deserialize)]
#[serde(try_from = "Value")]
enum Step {
    EntityTransactions(EntityStep),
    Daily(DailyStep),
}

#[derive(Debug, Deserialize)]
struct EntityStep {
    updates: Vec<SpendUpdateQueueItem>,
}

#[derive(Debug, Deserialize)]
struct DailyStep {
    updates: Vec<DailySpendUpdate>,
}

impl TryFrom<Value> for Step {
    type Error = serde_json::Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let Some(kind) = value.get("kind").and_then(Value::as_str) else {
            return Err(serde_json::Error::custom(
                "a spend queue step names no kind",
            ));
        };
        match kind {
            "entity_transactions" => serde_json::from_value(value).map(Self::EntityTransactions),
            "daily" => serde_json::from_value(value).map(Self::Daily),
            other => Err(serde_json::Error::custom(format!(
                "unknown spend queue step kind: {other}"
            ))),
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = BTreeMap::new();
        for (name, step) in &input.steps {
            results.insert(name.clone(), run_step(step)?);
        }
        serde_json::to_value(results).map_err(failure)
    }
}

fn run_step(step: &Step) -> Result<Value, Box<HelperError>> {
    match step {
        Step::EntityTransactions(entity) => match aggregate_db_transactions(&entity.updates) {
            Ok(transactions) => serde_json::to_value(transactions).map_err(failure),
            // A cost sum betterlite cannot represent is reported as invalid, where the source would carry an infinity.
            Err(_) => Ok(serde_json::json!({ "invalid": true })),
        },
        Step::Daily(daily) => match aggregate_daily_rows(&daily.updates) {
            Ok(aggregated) => serde_json::to_value(aggregated).map_err(failure),
            // The source raises where a payload cannot be added, and Python reports that step as invalid.
            Err(_) => Ok(serde_json::json!({ "invalid": true })),
        },
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "SpendQueueMerge".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

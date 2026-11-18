use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_db::spend_queues::daily::{
    DailySpendUpdate, aggregate_daily_spend_update_transactions,
};
use betterlite_db::spend_queues::entity::{
    SpendUpdateQueueItem, aggregate_db_spend_update_transactions,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: BTreeMap<String, Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    EntityTransactions { updates: Vec<SpendUpdateQueueItem> },
    Daily { updates: Vec<DailySpendUpdate> },
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

fn run_step(step: &Step) -> Result<serde_json::Value, Box<HelperError>> {
    match step {
        Step::EntityTransactions { updates } => {
            match aggregate_db_spend_update_transactions(updates) {
                Ok(transactions) => serde_json::to_value(transactions).map_err(failure),
                // A cost sum betterlite cannot represent is reported as invalid, where the source would carry an infinity.
                Err(_) => Ok(serde_json::json!({ "invalid": true })),
            }
        }
        Step::Daily { updates } => match aggregate_daily_spend_update_transactions(updates) {
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

use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_db::model_usage::ModelUsagePayload;
use betterlite_db::model_usage::rollup::build_model_usage_transaction;
use betterlite_db::window_queue::{
    WindowSpendTransaction, aggregate_window_spend, to_wire_payload,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    window_transactions: Option<WindowStep>,
    #[serde(default)]
    wire_payload: Option<WireStep>,
    #[serde(default)]
    model_usage: Option<UsageStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowStep {
    updates: Vec<Vec<WindowSpendTransaction>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireStep {
    transaction: WindowSpendTransaction,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageStep {
    payload: ModelUsagePayload,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = BTreeMap::new();
        if let Some(step) = &input.window_transactions {
            let aggregated = aggregate_window_spend(&step.updates).map_err(failure)?;
            results.insert(
                "window_transactions",
                serde_json::to_value(aggregated).map_err(failure)?,
            );
        }
        if let Some(step) = &input.wire_payload {
            results.insert(
                "wire_payload",
                serde_json::to_value(to_wire_payload(&step.transaction)).map_err(failure)?,
            );
        }
        if let Some(step) = &input.model_usage {
            let transaction = build_model_usage_transaction(&step.payload);
            results.insert(
                "model_usage",
                serde_json::to_value(transaction).map_err(failure)?,
            );
        }
        serde_json::to_value(results).map_err(failure)
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "QueueRollup".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

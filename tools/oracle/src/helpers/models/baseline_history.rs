use betterlite_models::baseline_history::error::BaselineHistoryError;
use betterlite_models::baseline_history::model::{BaselineHistory, BaselineObservation};
use betterlite_models::baseline_history::replay::advance_baseline_history;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) history: BaselineHistory,
    pub(crate) groups: Vec<Vec<BaselineObservation>>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let mut history = input.history.clone();
        let mut steps = Vec::with_capacity(input.groups.len());
        for group in &input.groups {
            let (next, estimates) =
                advance_baseline_history(&history, group).map_err(baseline_history_error)?;
            history = next;
            steps.push(json!({ "estimates": estimates, "history": history }));
        }
        Ok(json!({ "steps": steps }))
    }
}

fn baseline_history_error(error: BaselineHistoryError) -> Box<HelperError> {
    let class = match error {
        BaselineHistoryError::CounterOverflow => "CounterOverflow",
        BaselineHistoryError::TimestampOutOfRange => "TimestampOutOfRange",
        BaselineHistoryError::InvalidTtl => "InvalidTtl",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

mod tests;

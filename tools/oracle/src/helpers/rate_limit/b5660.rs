use betterlite_core::routes::CallType;
use betterlite_rate_limit::limiter_assembly::selection::{LimiterChoice, select_call_type_limiter};
use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    call_type: CallType,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let choice = match select_call_type_limiter(input.call_type) {
            LimiterChoice::Main => "main",
            LimiterChoice::Batch => "batch",
        };
        Ok(serde_json::json!({ "choice": choice }))
    }
}

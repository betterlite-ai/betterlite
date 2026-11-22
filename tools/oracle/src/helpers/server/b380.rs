use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::shadow_eval_models::ShadowEvalModelError;
use betterlite_server::shadow_eval_models::job::ShadowJob;
use betterlite_server::shadow_eval_models::leg::ShadowLeg;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    job: ShadowJob,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answer(input).or_value_error()
    }
}

// the response LiteLLM's model_dump_json writes, plus the budget_spent property that dump cannot carry
fn answer(input: &Input) -> Result<Value, ShadowEvalModelError> {
    let response = input.job.response(Utc::now())?;
    let budget_spent: Vec<bool> = input
        .job
        .targets
        .iter()
        .map(ShadowLeg::budget_spent)
        .collect();
    Ok(json!({"response": response, "budget_spent": budget_spent}))
}

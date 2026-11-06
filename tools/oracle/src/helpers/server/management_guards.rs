use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

mod calls;
mod input;
mod outcome;

use calls::{
    caller_id_value, duration_value, guardrail_value, metadata_value, passthrough_value,
    spend_value, team_budget_value, update_value,
};
use input::Call;
use outcome::{Outcome, into_outcome};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let outcomes: Vec<Outcome> = input.calls.iter().map(run_guard).collect();
        serde_json::to_value(outcomes).map_err(|source| {
            Box::new(HelperError {
                class: "Output".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}

fn run_guard(call: &Call) -> Outcome {
    match call {
        Call::ValidateFiniteSpend(input) => into_outcome(spend_value(input)),
        Call::ValidateBudgetDuration(input) => into_outcome(duration_value(input)),
        Call::ValidateTeamModelMaxBudget(input) => into_outcome(team_budget_value(input)),
        Call::CallerUserIdForNonAdmin(input) => into_outcome(caller_id_value(input)),
        Call::CheckPassthroughRoutes(input) => into_outcome(passthrough_value(input)),
        Call::CheckGuardrailOptOut(input) => into_outcome(guardrail_value(input)),
        Call::SetObjectMetadataField(input) => into_outcome(Ok(metadata_value(input))),
        Call::UpdateMetadataField(input) => {
            into_outcome(update_value(&input.updated_kv, Some(&input.field_name)))
        }
        Call::UpdateMetadataFields(input) => into_outcome(update_value(&input.updated_kv, None)),
    }
}

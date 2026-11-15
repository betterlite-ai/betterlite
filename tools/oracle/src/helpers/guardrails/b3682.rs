use betterlite_guardrails::guardrail_registry::stable_id::stable_guardrail_id;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StableIdInput {
    function: Function,
    guardrail_name: String,
    existing_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    StableGuardrailId,
}

impl Helper for Op {
    type Input = StableIdInput;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::StableGuardrailId => {
                let id = stable_guardrail_id(&input.guardrail_name, &|candidate: &str| {
                    input
                        .existing_ids
                        .iter()
                        .any(|existing| existing.as_str() == candidate)
                });
                Ok(serde_json::json!({ "id": id }))
            }
        }
    }
}

use std::collections::BTreeSet;

use super::types::{PolicyValidationError, PolicyValidationErrorType};
use crate::values::Policy;

#[derive(Clone, Copy)]
enum GuardrailLocation {
    Add,
    Remove,
    PipelineAddList(usize),
    PipelineRegistry(usize),
}

fn guardrail_error(
    policy_name: &str,
    guardrail: &str,
    location: GuardrailLocation,
) -> PolicyValidationError {
    let (message, field) = match location {
        GuardrailLocation::Add => (
            format!("Guardrail '{guardrail}' not found in guardrail registry"),
            "guardrails.add",
        ),
        GuardrailLocation::Remove => (
            format!("Guardrail '{guardrail}' in remove list not found in guardrail registry"),
            "guardrails.remove",
        ),
        GuardrailLocation::PipelineAddList(index) => (
            format!(
                "Pipeline step {index} guardrail '{guardrail}' is not in the policy's guardrails.add list"
            ),
            "pipeline.steps",
        ),
        GuardrailLocation::PipelineRegistry(index) => (
            format!(
                "Pipeline step {index} guardrail '{guardrail}' not found in guardrail registry"
            ),
            "pipeline.steps",
        ),
    };
    PolicyValidationError {
        policy_name: policy_name.to_owned(),
        error_type: PolicyValidationErrorType::InvalidGuardrail,
        message,
        field: Some(field.to_owned()),
        value: Some(guardrail.to_owned()),
    }
}

pub(super) fn validate_guardrails(
    policy_name: &str,
    policy: &Policy,
    available_guardrails: &BTreeSet<String>,
) -> (Vec<PolicyValidationError>, Vec<PolicyValidationError>) {
    let registry_known = !available_guardrails.is_empty();
    let guardrails_add = policy.guardrails.get_add();
    let mut errors: Vec<PolicyValidationError> = guardrails_add
        .iter()
        .filter(|guardrail| registry_known && !available_guardrails.contains(*guardrail))
        .map(|guardrail| guardrail_error(policy_name, guardrail.as_str(), GuardrailLocation::Add))
        .collect();
    let warnings = policy
        .guardrails
        .get_remove()
        .iter()
        .filter(|guardrail| registry_known && !available_guardrails.contains(*guardrail))
        .map(|guardrail| {
            guardrail_error(policy_name, guardrail.as_str(), GuardrailLocation::Remove)
        })
        .collect();
    if let Some(pipeline) = &policy.pipeline {
        for (index, step) in pipeline.steps.iter().enumerate() {
            if !guardrails_add.contains(&step.guardrail) {
                errors.push(guardrail_error(
                    policy_name,
                    &step.guardrail,
                    GuardrailLocation::PipelineAddList(index),
                ));
            }
            if registry_known && !available_guardrails.contains(&step.guardrail) {
                errors.push(guardrail_error(
                    policy_name,
                    &step.guardrail,
                    GuardrailLocation::PipelineRegistry(index),
                ));
            }
        }
    }
    (errors, warnings)
}

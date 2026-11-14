use std::collections::{BTreeMap, BTreeSet};

use super::PolicyValidator;
use super::inheritance::validate_inheritance_chain;
use super::lookup::GuardrailRegistry;
use super::types::{PolicyValidationError, PolicyValidationErrorType, PolicyValidationResponse};
use crate::values::Policy;

impl<Guardrails: GuardrailRegistry, Database, Router>
    PolicyValidator<Guardrails, Database, Router>
{
    #[must_use]
    pub fn get_available_guardrails(&self) -> BTreeSet<String> {
        match self.guardrails.list_guardrail_names() {
            Ok(names) => names.into_iter().filter(|name| !name.is_empty()).collect(),
            Err(error) => {
                tracing::warn!("Could not get guardrails from registry: {error}");
                BTreeSet::new()
            }
        }
    }

    // validate_db is accepted and never read, as in LiteLLM
    #[must_use]
    pub fn validate_policies(
        &self,
        policies: &BTreeMap<String, Policy>,
        _validate_db: bool,
    ) -> PolicyValidationResponse {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let available_guardrails = self.get_available_guardrails();
        for (policy_name, policy) in policies {
            let (add_errors, remove_warnings) =
                validate_guardrail_lists(policy_name, policy, &available_guardrails);
            errors.extend(add_errors);
            warnings.extend(remove_warnings);
            errors.extend(validate_pipeline(
                policy_name,
                policy,
                &available_guardrails,
            ));
            errors.extend(validate_inheritance_chain(policy_name, policies));
        }
        PolicyValidationResponse {
            valid: errors.is_empty(),
            errors,
            warnings,
        }
    }
}

fn validate_guardrail_lists(
    policy_name: &str,
    policy: &Policy,
    available_guardrails: &BTreeSet<String>,
) -> (Vec<PolicyValidationError>, Vec<PolicyValidationError>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for guardrail in policy.guardrails.get_add() {
        if !available_guardrails.is_empty() && !available_guardrails.contains(&guardrail) {
            errors.push(PolicyValidationError {
                policy_name: policy_name.to_owned(),
                error_type: PolicyValidationErrorType::InvalidGuardrail,
                message: format!("Guardrail '{guardrail}' not found in guardrail registry"),
                field: Some("guardrails.add".to_owned()),
                value: Some(guardrail),
            });
        }
    }
    for guardrail in policy.guardrails.get_remove() {
        if !available_guardrails.is_empty() && !available_guardrails.contains(&guardrail) {
            warnings.push(PolicyValidationError {
                policy_name: policy_name.to_owned(),
                error_type: PolicyValidationErrorType::InvalidGuardrail,
                message: format!(
                    "Guardrail '{guardrail}' in remove list not found in guardrail registry"
                ),
                field: Some("guardrails.remove".to_owned()),
                value: Some(guardrail),
            });
        }
    }
    (errors, warnings)
}

fn validate_pipeline(
    policy_name: &str,
    policy: &Policy,
    available_guardrails: &BTreeSet<String>,
) -> Vec<PolicyValidationError> {
    let mut errors = Vec::new();
    let Some(pipeline) = &policy.pipeline else {
        return errors;
    };
    let guardrails_add = policy.guardrails.get_add();
    for (index, step) in pipeline.steps.iter().enumerate() {
        if !guardrails_add.contains(&step.guardrail) {
            errors.push(PolicyValidationError {
                policy_name: policy_name.to_owned(),
                error_type: PolicyValidationErrorType::InvalidGuardrail,
                message: format!(
                    "Pipeline step {index} guardrail '{}' is not in the policy's guardrails.add list",
                    step.guardrail
                ),
                field: Some("pipeline.steps".to_owned()),
                value: Some(step.guardrail.clone()),
            });
        }
        if !available_guardrails.is_empty() && !available_guardrails.contains(&step.guardrail) {
            errors.push(PolicyValidationError {
                policy_name: policy_name.to_owned(),
                error_type: PolicyValidationErrorType::InvalidGuardrail,
                message: format!(
                    "Pipeline step {index} guardrail '{}' not found in guardrail registry",
                    step.guardrail
                ),
                field: Some("pipeline.steps".to_owned()),
                value: Some(step.guardrail.clone()),
            });
        }
    }
    errors
}

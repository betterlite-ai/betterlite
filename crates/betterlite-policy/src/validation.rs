pub mod config;
pub mod error;
pub mod guardrails;
pub mod inheritance;
pub mod lookup;
pub mod scope;
pub mod types;

mod tests;

use std::collections::{BTreeMap, BTreeSet};

use crate::pipeline::json_value::JsonValue;
use crate::values::Policy;

use self::inheritance::validate_inheritance_chain;
use self::lookup::GuardrailRegistry;
use self::types::PolicyValidationResponse;

#[derive(Debug, Clone)]
pub struct PolicyValidator<Guardrails, Database, Router> {
    pub guardrails: Guardrails,
    pub database: Option<Database>,
    pub router: Option<Router>,
}

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
            let (guardrail_errors, guardrail_warnings) =
                guardrails::validate_guardrails(policy_name, policy, &available_guardrails);
            errors.extend(guardrail_errors);
            warnings.extend(guardrail_warnings);
            errors.extend(validate_inheritance_chain(policy_name, policies));
        }
        PolicyValidationResponse {
            valid: errors.is_empty(),
            errors,
            warnings,
        }
    }

    #[must_use]
    pub fn validate_policy_config(
        &self,
        policy_config: &BTreeMap<String, JsonValue>,
        validate_db: bool,
    ) -> PolicyValidationResponse {
        match config::parse_policies(policy_config) {
            Ok(policies) => self.validate_policies(&policies, validate_db),
            Err(errors) => PolicyValidationResponse {
                valid: false,
                errors,
                warnings: Vec::new(),
            },
        }
    }
}

use std::collections::BTreeMap;

use super::error::PolicyParseError;
use super::types::{PolicyValidationError, PolicyValidationErrorType};
use crate::pipeline::json_value::JsonValue;
use crate::registry::{RawPolicy, parse_policy};
use crate::values::Policy;

fn parse_policy_value(policy_name: &str, data: &JsonValue) -> Result<Policy, PolicyParseError> {
    let text = serde_json::to_string(data)?;
    let raw: RawPolicy = serde_json::from_str(&text)?;
    Ok(parse_policy(policy_name, raw)?)
}

pub(super) fn parse_policies(
    policy_config: &BTreeMap<String, JsonValue>,
) -> Result<BTreeMap<String, Policy>, Vec<PolicyValidationError>> {
    let mut errors = Vec::new();
    let mut policies = BTreeMap::new();
    for (policy_name, data) in policy_config {
        match parse_policy_value(policy_name, data) {
            Ok(policy) => {
                policies.insert(policy_name.clone(), policy);
            }
            Err(error) => errors.push(PolicyValidationError {
                policy_name: policy_name.clone(),
                error_type: PolicyValidationErrorType::InvalidSyntax,
                message: format!("Failed to parse policy: {error}"),
                field: None,
                value: None,
            }),
        }
    }
    if errors.is_empty() {
        return Ok(policies);
    }
    Err(errors)
}

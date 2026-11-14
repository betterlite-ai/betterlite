use indexmap::IndexMap;

use super::types::{PolicyValidationError, PolicyValidationErrorType};
use crate::values::Policy;

pub const MAX_INHERITANCE_DEPTH: usize = 100;

#[must_use]
pub fn validate_inheritance_chain(
    policy_name: &str,
    policies: &IndexMap<String, Policy>,
) -> Option<PolicyValidationError> {
    let mut visited: Vec<&str> = Vec::new();
    let mut current = policy_name;
    loop {
        let too_deep = visited.len() >= MAX_INHERITANCE_DEPTH;
        if too_deep || visited.contains(&current) {
            let message = if too_deep {
                format!(
                    "Inheritance chain too deep (exceeded max depth of {MAX_INHERITANCE_DEPTH})"
                )
            } else {
                format!(
                    "Circular inheritance detected: {} -> {current}",
                    visited.join(" -> ")
                )
            };
            return Some(PolicyValidationError {
                policy_name: current.to_owned(),
                error_type: PolicyValidationErrorType::CircularInheritance,
                message,
                field: Some("inherit".to_owned()),
                value: None,
            });
        }
        let policy = policies.get(current)?;
        let parent = policy.inherit.as_deref().filter(|name| !name.is_empty())?;
        if !policies.contains_key(parent) {
            return Some(PolicyValidationError {
                policy_name: current.to_owned(),
                error_type: PolicyValidationErrorType::InvalidInheritance,
                message: format!("Parent policy '{parent}' not found"),
                field: Some("inherit".to_owned()),
                value: Some(parent.to_owned()),
            });
        }
        visited.push(current);
        current = parent;
    }
}

use std::collections::BTreeMap;

use betterlite_policy::registry::{PolicyRegistry, RawPolicy};

use crate::helpers::HelperError;

pub(super) fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn loaded(
    policies: &BTreeMap<String, RawPolicy>,
) -> Result<PolicyRegistry, Box<HelperError>> {
    let mut registry = PolicyRegistry::default();
    registry
        .load_policies(policies.clone())
        .map_err(|error| invalid_input(error.to_string()))?;
    Ok(registry)
}

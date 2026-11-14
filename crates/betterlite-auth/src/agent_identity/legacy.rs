use std::collections::BTreeMap;

use super::error::AgentIdentityError;

pub const LEGACY_IDENTITY_MESSAGE: &str = "litellm_params.identity is not supported: bind an Entra application through the top-level identity field";
pub const LEGACY_IDENTITY_STATUS: u16 = 400;

#[must_use]
pub fn has_legacy_identity<Value>(params: Option<&BTreeMap<String, Value>>) -> bool {
    if let Some(fields) = params {
        return fields.contains_key("identity");
    }
    false
}

pub fn reject_legacy_identity<Value>(
    params: Option<&BTreeMap<String, Value>>,
) -> Result<(), AgentIdentityError> {
    if !has_legacy_identity(params) {
        return Ok(());
    }
    Err(AgentIdentityError {
        status: LEGACY_IDENTITY_STATUS,
        message: String::from(LEGACY_IDENTITY_MESSAGE),
    })
}

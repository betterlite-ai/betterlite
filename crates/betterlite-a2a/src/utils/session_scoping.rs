use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::completion_bridge::wire::MessageSendParams;

mod tests;

// the prefix keeps the agent backend from ever seeing the raw principal
const PRINCIPAL_PREFIX_CHARS: usize = 16;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SessionScopeError {
    #[error("the A2A contextId must be a string")]
    ContextIdType,
}

pub fn get_session_id_from_a2a_params(
    params: &MessageSendParams,
) -> Result<Option<String>, SessionScopeError> {
    let Some(value) = params.message.extra.get("contextId") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(|context_id| Some(context_id.to_owned()))
        .ok_or(SessionScopeError::ContextIdType)
}

#[must_use]
pub fn scope_session_to_principal(session_id: &str, principal: Option<&str>) -> String {
    let Some(token) = principal.filter(|value| !value.is_empty()) else {
        return session_id.to_owned();
    };
    let digest = hex::encode(Sha256::digest(token.as_bytes()));
    let prefix: String = digest.chars().take(PRINCIPAL_PREFIX_CHARS).collect();
    format!("{prefix}-{session_id}")
}

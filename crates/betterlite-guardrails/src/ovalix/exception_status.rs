use thiserror::Error;

use super::checkpoint::CheckpointError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct OvalixBlockedException {
    pub guardrail_name: String,
    pub message: String,
}

impl OvalixBlockedException {
    pub const STATUS_CODE: u16 = 400;
    pub const BLOCKED_CONTENT: bool = true;
}

#[derive(Debug, Error)]
#[error("Ovalix guardrail error: {source}")]
pub struct GuardrailRaisedException {
    pub guardrail_name: String,
    #[source]
    pub source: CheckpointError,
}

impl GuardrailRaisedException {
    pub const STATUS_CODE: u16 = 400;
    pub const BLOCKED_CONTENT: bool = false;
}

mod tests;

use thiserror::Error;

use super::slot::FailureReason;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DiscoveryUnavailable {
    #[error("OAuth metadata discovery {reason} for MCP server '{server_ref}'")]
    Failed {
        reason: FailureReason,
        server_ref: String,
    },
    #[error("OAuth metadata discovery changed repeatedly; retry shortly")]
    ChangedRepeatedly,
}

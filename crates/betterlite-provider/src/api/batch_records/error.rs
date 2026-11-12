use thiserror::Error;

use crate::api::file_contracts::error::BaseLlmException;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BatchContractError {
    #[error(transparent)]
    Upstream(#[from] BaseLlmException),
}

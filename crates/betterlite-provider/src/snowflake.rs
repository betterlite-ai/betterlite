//! Snowflake Cortex REST API transforms: OpenAI chat completions, the Claude Messages dialect and embeddings.

pub mod config;
pub mod embedding;
pub mod messages;
pub mod request;
pub mod response;
pub mod stream;

use thiserror::Error;

// LiteLLM's SnowflakeException subclasses BaseLLMException and adds no member, so B2202's error is the whole class
pub type SnowflakeException = crate::api::file_contracts::error::BaseLlmException;

// LiteLLM raises these as ValueError texts from SnowflakeBaseConfig._get_api_base and validate_environment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SnowflakeError {
    #[error("Missing snowflake account_id")]
    MissingAccountId,
    #[error("Invalid account_id format")]
    InvalidAccountId,
    #[error("Missing Snowflake JWT key")]
    MissingJwtKey,
    #[error("account_id must be a string")]
    AccountIdType,
}

#[cfg(test)]
pub(crate) mod tests;

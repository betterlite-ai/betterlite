use thiserror::Error;

pub mod config;
pub mod embedding;
pub mod messages;
pub mod request;
pub mod response;
pub mod stream;

pub type SnowflakeException = crate::api::file_contracts::error::BaseLlmException;

// The fetchers are all async-only, so the three call sites share one bound instead of repeating it
pub trait Fetcher: betterlite_prompts::media::inline::RemoteMediaFetcher + Sync {}
impl<T> Fetcher for T where T: betterlite_prompts::media::inline::RemoteMediaFetcher + Sync {}

#[derive(Debug, PartialEq, Eq, Error)]
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

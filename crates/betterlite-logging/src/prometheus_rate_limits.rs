use std::collections::BTreeMap;

use async_trait::async_trait;

pub mod deployment;
pub mod error;
pub mod gauge;
pub mod headers;
pub mod key_and_team;
pub mod metric;
pub mod router_remaining;
pub mod virtual_key;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum RemainingUsageError {
    #[error("the router could not read the model group usage")]
    Unavailable,
}

// LiteLLM reaches the proxy's llm_router global; betterlite passes the reader in, and B8655 implements it over B847
#[async_trait]
pub trait RemainingModelGroupUsage: Send + Sync {
    // ast-grep-ignore: no-long-names
    async fn get_remaining_model_group_usage(
        &self,
        model_group: &str,
    ) -> Result<BTreeMap<String, i64>, RemainingUsageError>;
}

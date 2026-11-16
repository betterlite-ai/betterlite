use betterlite_db::postgres_store::errors::DbError;
use thiserror::Error;

// every repository failure keeps its cause; the store turns each one into LiteLLM's policy_unavailable
#[derive(Debug, Error)]
pub enum AgentStoreError {
    #[error("the agent identity database call failed")]
    Database(#[source] DbError),
    #[error("the agent row could not be decoded")]
    Row(#[source] serde_json::Error),
}

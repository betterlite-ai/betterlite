use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentCredentialsError {
    #[error("the agent credential JSON could not be built")]
    Serialize(#[source] serde_json::Error),
}

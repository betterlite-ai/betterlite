use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::agent_requests::error::AgentRequestError;
use crate::credentials::error::CredentialsError;

use super::decode::AgentRunDecodeError;

pub const RUN_FAILED_STATUS: u16 = 500;
pub const RUN_TIMED_OUT_STATUS: u16 = 408;

// AzureAIAgentsError is a BaseLLMException, and the missing-token failure is a plain ValueError
#[derive(Debug, Error)]
pub enum AgentRunError {
    #[error("Run {status}: {message}")]
    RunFailed { status: String, message: String },
    #[error("Run timed out waiting for completion")]
    TimedOut,
    #[error(
        "api_key (Azure AD token) is required for Azure Foundry Agents. Either pass api_key directly, or set AZURE_TENANT_ID, AZURE_CLIENT_ID, and AZURE_CLIENT_SECRET environment variables for Service Principal auth. Manual token: az account get-access-token --resource 'https://ai.azure.com'"
    )]
    MissingToken,
    #[error("the agent request body could not be serialized")]
    Body(#[source] serde_json::Error),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Request(#[from] AgentRequestError),
    #[error(transparent)]
    Decode(#[from] AgentRunDecodeError),
    #[error(transparent)]
    Credentials(#[from] CredentialsError),
}

impl AgentRunError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::RunFailed { .. } => Some(RUN_FAILED_STATUS),
            Self::TimedOut => Some(RUN_TIMED_OUT_STATUS),
            Self::Request(error) => error.status(),
            Self::MissingToken
            | Self::Body(_)
            | Self::Transport(_)
            | Self::Decode(_)
            | Self::Credentials(_) => None,
        }
    }
}

use betterlite_http::error::HttpError;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;

// AzureAIAgentsError is a BaseLLMException: a status and a message, with the headers get_error_class receives dropped
#[derive(Debug, Error)]
pub enum AgentRequestError {
    #[error(
        "api_base is required for Azure AI Agents. Set it via AZURE_AI_API_BASE env var or api_base parameter."
    )]
    ApiBaseRequired,
    #[error("the {field} field must be a string")]
    FieldType { field: &'static str },
    #[error("{message}")]
    Upstream { status: u16, message: String },
    #[error(transparent)]
    Segment(#[from] HttpError),
    #[error("cannot read the {name} secret")]
    Secret {
        name: &'static str,
        #[source]
        source: SecretLookupError,
    },
}

impl AgentRequestError {
    // the Python class each failure raises: get_error_class builds AzureAIAgentsError, the other places raise ValueError
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Upstream { .. } => "AzureAIAgentsError",
            Self::ApiBaseRequired | Self::FieldType { .. } | Self::Segment(_) => "ValueError",
            Self::Secret { .. } => "ConfigError",
        }
    }

    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Upstream { status, .. } => Some(*status),
            Self::ApiBaseRequired
            | Self::FieldType { .. }
            | Self::Segment(_)
            | Self::Secret { .. } => None,
        }
    }
}

#[must_use]
pub const fn error_for_status(status: u16, message: String) -> AgentRequestError {
    // get_error_class ignores the headers it is handed, so the error keeps only the status and the message
    AgentRequestError::Upstream { status, message }
}

pub fn check_response(
    status: u16,
    body: &str,
    expected_codes: &[u16],
    error_msg: &str,
) -> Result<(), AgentRequestError> {
    if expected_codes.contains(&status) {
        return Ok(());
    }
    Err(AgentRequestError::Upstream {
        status,
        message: format!("{error_msg}: {body}"),
    })
}

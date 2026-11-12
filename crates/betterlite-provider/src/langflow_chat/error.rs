use betterlite_http::error::HttpError;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;

pub const FLOW_ID_IN_PARAMS_MESSAGE: &str =
    "flow_id cannot be set via request parameters; use model langflow/{flow_id}";
pub const FLOW_ID_REQUIRED_MESSAGE: &str = "flow_id is required; use model langflow/{flow_id}";
pub const TWEAKS_MESSAGE: &str = "tweaks cannot be set via request parameters; they would override the operator-configured LangFlow flow components";
pub const NO_MESSAGE_MESSAGE: &str = "Could not extract a message from the LangFlow response; ensure the flow ends in a Chat Output component";

#[derive(Debug, Error)]
pub enum LangFlowError {
    // LangFlowError, the class get_error_class returns: the provider's status and text
    #[error("{message}")]
    Api { status_code: u16, message: String },
    // LiteLLM's ValueError when no base reached get_complete_url
    #[error(
        "api_base is required for LangFlow. Set it via LANGFLOW_API_BASE env var or api_base parameter."
    )]
    ApiBaseRequired,
    #[error("the {name} setting could not be read: {source}")]
    Environment {
        name: &'static str,
        #[source]
        source: SecretLookupError,
    },
    // the flow id from the model cannot be a dot path segment, so it never reaches the run URL
    #[error("the LangFlow flow id {flow_id} cannot be written as a URL path segment: {source}")]
    FlowId {
        flow_id: String,
        #[source]
        source: HttpError,
    },
}

impl LangFlowError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Api { status_code, .. } => Some(*status_code),
            Self::ApiBaseRequired | Self::Environment { .. } | Self::FlowId { .. } => None,
        }
    }
}

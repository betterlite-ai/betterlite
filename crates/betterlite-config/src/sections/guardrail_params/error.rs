use betterlite_env::EnvError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GuardrailParamsError {
    #[error("invalid field {field}")]
    InvalidField { field: String },
    #[error("missing field {field}")]
    MissingField { field: String },
    #[error("sensitive_data_route_to_model must be set when on_sensitive_data='route'")]
    RouteModelMissing,
    #[error("on_violation='{value}' is only supported by guardrail='mcp_security'")]
    ViolationNeedsMcpSecurity { value: String },
    #[error(
        "checks as a list is only supported by guardrail='decision_model', got guardrail='{guardrail}'"
    )]
    ChecksNeedDecisionModel { guardrail: String },
    #[error("timeout must be numeric")]
    InvalidTimeout,
    #[error("the body has data after the JSON value")]
    TrailingData,
    #[error(transparent)]
    Environment(#[from] EnvError),
    #[error("the parameters could not be written")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl From<serde_path_to_error::Error<serde_json::Error>> for GuardrailParamsError {
    fn from(error: serde_path_to_error::Error<serde_json::Error>) -> Self {
        Self::InvalidField {
            field: error.path().to_string(),
        }
    }
}

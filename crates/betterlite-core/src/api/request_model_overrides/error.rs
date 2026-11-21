use serde::Serialize;
use thiserror::Error;

// what the handler mapper reads; LiteLLM raises a bare ValueError here, so the status and wire fields are ours
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorFacts {
    pub status: u16,
    #[serde(rename = "type")]
    pub error_type: &'static str,
    pub param: Option<String>,
    pub code: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RequestModelOverridesError {
    #[error("the field {field} must be {expected}")]
    InvalidField {
        field: String,
        expected: &'static str,
    },
    #[error("the request must carry the enforced param {param}")]
    MissingEnforcedParam { param: String },
    #[error("the field {field} must be an object to carry the enforced param {param}")]
    EnforcedParamParentNotObject { field: String, param: String },
    #[error("enforced params needs an enterprise license")]
    NotPremiumUser,
}

impl RequestModelOverridesError {
    #[must_use]
    pub fn facts(&self) -> ErrorFacts {
        match self {
            Self::InvalidField { field, .. } => ErrorFacts {
                status: 400,
                error_type: "invalid_request_error",
                param: Some(field.clone()),
                code: "400",
            },
            Self::MissingEnforcedParam { param } => ErrorFacts {
                status: 400,
                error_type: "invalid_request_error",
                param: Some(param.clone()),
                code: "400",
            },
            Self::EnforcedParamParentNotObject { field, .. } => ErrorFacts {
                status: 400,
                error_type: "invalid_request_error",
                param: Some(field.clone()),
                code: "400",
            },
            Self::NotPremiumUser => ErrorFacts {
                status: 403,
                error_type: "permission_error",
                param: None,
                code: "403",
            },
        }
    }
}

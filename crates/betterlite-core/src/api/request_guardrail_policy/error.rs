use serde::Serialize;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;

#[derive(Debug, Error)]
pub enum RequestGuardrailPolicyError {
    #[error(
        "guardrails and policies from key, team or project metadata need an enterprise license"
    )]
    NotPremium,
    #[error("'{field}' must be {expected}")]
    NotTyped {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the request metadata bucket must be a JSON object")]
    MetadataBucket,
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
}

// what the handler mapper reads off the exception; the HTTP wire envelope is the mapper's answer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GuardrailPolicyErrorFacts {
    pub status: u16,
    #[serde(rename = "type")]
    pub error_type: &'static str,
    pub param: Option<&'static str>,
    pub code: &'static str,
}

impl RequestGuardrailPolicyError {
    #[must_use]
    pub const fn facts(&self) -> GuardrailPolicyErrorFacts {
        match self {
            Self::NotPremium => GuardrailPolicyErrorFacts {
                status: 403,
                error_type: "permission_error",
                param: None,
                code: "403",
            },
            Self::NotTyped { field, .. } => GuardrailPolicyErrorFacts {
                status: 400,
                error_type: "invalid_request_error",
                param: Some(field),
                code: "400",
            },
            Self::MetadataBucket => GuardrailPolicyErrorFacts {
                status: 400,
                error_type: "invalid_request_error",
                param: Some("metadata"),
                code: "400",
            },
            Self::Limit(error) => {
                let fields = error.fields();
                GuardrailPolicyErrorFacts {
                    status: fields.status,
                    error_type: fields.error_type,
                    param: Some(fields.param),
                    code: fields.code,
                }
            }
        }
    }
}

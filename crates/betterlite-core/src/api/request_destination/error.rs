use serde::Serialize;
use thiserror::Error;

use crate::api::request_destination::destination::DestinationField;

// the FastAPI detail object the exception constructor passes to the handler mapper, which keeps it in provider_specific_fields
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExceptionDetail {
    pub error: &'static str,
    pub param: DestinationField,
}

// what the handler mapper reads off an exception; the HTTP wire envelope is the mapper's answer, not this record
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ErrorFacts {
    pub status: u16,
    #[serde(rename = "type")]
    pub error_type: &'static str,
    pub param: Option<&'static str>,
    pub code: &'static str,
    pub detail: Option<ExceptionDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RequestDestinationError {
    #[error(
        "URL-valued '{field}' is not allowed; configure a custom endpoint with api_base or allowlist the destination host"
    )]
    UrlValuedDestination { field: DestinationField },
    #[error("the request names no model and no default model is configured")]
    MissingModel,
    #[error("'model' must be a string")]
    ModelNotText,
}

impl RequestDestinationError {
    #[must_use]
    pub const fn facts(&self) -> ErrorFacts {
        match self {
            Self::UrlValuedDestination { field } => ErrorFacts {
                status: 400,
                error_type: "invalid_request_error",
                param: None,
                code: "400",
                detail: Some(ExceptionDetail {
                    error: "invalid_request",
                    param: *field,
                }),
            },
            Self::MissingModel => ErrorFacts {
                status: 400,
                error_type: "invalid_request_error",
                param: None,
                code: "400",
                detail: None,
            },
            Self::ModelNotText => ErrorFacts {
                status: 400,
                error_type: "bad_request_error",
                param: Some("model"),
                code: "400",
                detail: None,
            },
        }
    }
}

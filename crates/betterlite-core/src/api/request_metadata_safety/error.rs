use serde::Serialize;
use thiserror::Error;

use crate::api::request_metadata_safety::metadata::{JsonType, MetadataField};

// what the handler mapper reads off an exception; the HTTP wire envelope is the mapper's answer, not this record
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ErrorFacts {
    pub status: u16,
    #[serde(rename = "type")]
    pub error_type: &'static str,
    pub param: Option<&'static str>,
    pub code: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RequestMetadataSafetyError {
    #[error(
        "Invalid type for '{}': expected an object, but got {} instead.",
        .field.as_str(),
        .received.describe()
    )]
    MetadataNotObject {
        field: MetadataField,
        received: JsonType,
    },
}

impl RequestMetadataSafetyError {
    #[must_use]
    pub const fn facts(&self) -> ErrorFacts {
        match self {
            Self::MetadataNotObject { field, .. } => ErrorFacts {
                status: 400,
                error_type: "bad_request_error",
                param: Some(field.as_str()),
                code: "400",
            },
        }
    }
}

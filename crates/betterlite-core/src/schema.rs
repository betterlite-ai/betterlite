#![expect(
    clippy::disallowed_types,
    reason = "A JSON Schema is arbitrary JSON by definition: its keywords, nesting and types are caller data, not a record betterlite models"
)]

use betterlite_error::error::details::SchemaValidationDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use jsonschema::ValidationError;
use thiserror::Error;

mod bounds;
pub mod normalize;
pub mod validate;

mod tests;

// a schema or response past these bounds costs more work than one gateway call may spend on it
const DEFAULT_MAX_SCHEMA_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_DEPTH: usize = 100;
const DEFAULT_MAX_NODES: usize = 10_000;

pub type SchemaJson = serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaLimits {
    pub max_schema_bytes: usize,
    pub max_response_bytes: usize,
    pub max_depth: usize,
    pub max_nodes: usize,
}

impl Default for SchemaLimits {
    fn default() -> Self {
        Self {
            max_schema_bytes: DEFAULT_MAX_SCHEMA_BYTES,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_depth: DEFAULT_MAX_DEPTH,
            max_nodes: DEFAULT_MAX_NODES,
        }
    }
}

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("the schema is larger than the {limit} byte limit")]
    SchemaTooLarge { limit: usize },
    #[error("the schema has more than the {limit} node limit")]
    TooManyNodes { limit: usize },
    #[error("the schema nests deeper than the {limit} level limit")]
    SchemaTooDeep { limit: usize },
    #[error("the schema references the external document {reference}")]
    ExternalReference { reference: String },
    #[error("the response is {size} bytes, past the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("the response is not valid JSON")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the schema is not a valid JSON Schema")]
    InvalidSchema {
        #[source]
        source: ValidationError<'static>,
    },
    #[error("the response does not match the schema")]
    Mismatch {
        #[source]
        source: ValidationError<'static>,
    },
}

impl SchemaError {
    // LiteLLM raises JSONSchemaValidationError for a bad response and leaves every other schema failure uncaught
    #[must_use]
    pub fn classify(&self, schema: &SchemaJson, raw_response: &str) -> ErrorClass {
        match self {
            Self::InvalidJson { .. } | Self::Mismatch { .. } => {
                ErrorClass::JsonSchemaValidation(Box::new(SchemaValidationDetails {
                    model: String::new(),
                    provider: String::new(),
                    raw_response: raw_response.to_owned(),
                    schema: schema.to_string(),
                }))
            }
            Self::SchemaTooLarge { .. }
            | Self::TooManyNodes { .. }
            | Self::SchemaTooDeep { .. }
            | Self::ExternalReference { .. }
            | Self::ResponseTooLarge { .. }
            | Self::InvalidSchema { .. } => ErrorClass::InternalServer(Box::new(ErrorContext {
                message: self.to_string(),
                model: None,
                provider: None,
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: None,
                body_code: None,
            })),
        }
    }
}

pub mod callbacks;
pub mod credentials;
pub mod internal;
pub mod request;
pub mod snapshot;

use indexmap::IndexMap;
use std::fmt;

use betterlite_models::decisions::Json;
use betterlite_redact::error::RedactionError;

#[derive(Clone, PartialEq, Eq)]
pub struct CallbackMetadata {
    pub fields: IndexMap<String, Json>,
    pub skipped_fields: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CallbackMetadataError {
    #[error("invalid callback field {field}")]
    InvalidField { field: String },
    #[error("invalid privacy metadata: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error("callback param '{field}' contains an os.environ/ reference")]
    EnvironmentReference { field: String },
    #[error("request metadata cannot set trusted internal field '{field}'")]
    TrustedField { field: String },
    #[error("credential redaction could not initialize: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrivacyContext {
    pub message_logging_disabled: bool,
}

#[path = "logging_privacy/tests.rs"]
mod tests;

impl fmt::Debug for CallbackMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallbackMetadata")
            .field("fields", &self.fields.keys().collect::<Vec<_>>())
            .field("skipped_fields", &self.skipped_fields)
            .finish()
    }
}

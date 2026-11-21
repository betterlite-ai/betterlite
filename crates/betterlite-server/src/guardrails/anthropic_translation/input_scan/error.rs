use thiserror::Error;

use crate::guardrails::anthropic_translation::structured_write_back::error::StructuredWriteBackError;

#[derive(Debug, Error)]
pub enum StructuredMessagesError {
    #[error("the Anthropic messages could not be translated to chat messages")]
    Translation(#[from] StructuredWriteBackError),
    #[error("the translated chat request could not be read back as JSON")]
    Encoding(#[from] serde_json::Error),
}

// The one error for every move between provider JSON and the typed values around it
#[derive(Debug, Error)]
pub enum ConvertError {
    #[error("{target} could not be converted")]
    Convert {
        target: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

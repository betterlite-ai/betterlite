use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::assistant_records::error::class_name;
use crate::api::vector_file_http_read::error::ReadError;

#[derive(Debug, Error)]
pub enum VectorFileReadError {
    #[error("'{name}' is not a valid LlmProviders")]
    InvalidProvider { name: String },
    #[error("Vector store file {operation} is not supported for {provider}")]
    Unsupported {
        operation: &'static str,
        provider: String,
    },
    #[error("the {field} field must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
    #[error("{source}")]
    Read {
        provider: String,
        #[source]
        source: ReadError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
}

// The exception the SDK raises: its class, status and the type/param/code attributes it carries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkFailure {
    pub class: String,
    pub status: u16,
    pub kind: WireField,
    pub param: WireField,
    pub code: WireField,
    pub message: String,
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(error: &VectorFileReadError, redactor: &Redactor) -> SdkFailure {
    let (provider, status, class, message) = raised(error);
    let projected = dispatch(
        &DispatchInput::Raw(Box::new(RawException {
            model: String::new(),
            custom_llm_provider: provider.to_owned(),
            error_str: message.clone(),
            status_code: status,
            status_code_is_synthesized: false,
            message: Some(message.clone()),
            body: None,
            exception_type: class,
        })),
        redactor,
    );
    let metadata = projected.metadata();
    SdkFailure {
        class: class_name(&projected).to_owned(),
        status: metadata.status_code,
        kind: metadata.error_type,
        param: metadata.param,
        code: metadata
            .openai_code
            .map_or(WireField::Null, WireField::Text),
        message,
    }
}

// The provider, its status, the exception class exception_type saw and the text it mapped
fn raised(error: &VectorFileReadError) -> (&str, Option<u16>, String, String) {
    let (provider, status) = match error {
        VectorFileReadError::Read { provider, source } => (provider.as_str(), read_status(source)),
        VectorFileReadError::Unsupported { provider, .. } => (provider.as_str(), None),
        VectorFileReadError::InvalidProvider { .. }
        | VectorFileReadError::Field { .. }
        | VectorFileReadError::Logging(_) => ("", None),
    };
    // an SDK status error keeps its status and lets exception_type read the class from the status
    let class = if status.is_some() {
        String::new()
    } else {
        String::from("ValueError")
    };
    (provider, status, class, error.to_string())
}

const fn read_status(source: &ReadError) -> Option<u16> {
    if let ReadError::Provider(exception) = source {
        return Some(exception.status_code);
    }
    None
}

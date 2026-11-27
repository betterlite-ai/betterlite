use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::service::Operation;
use crate::api::assistant_records::error::class_name;
use crate::api::vector_file_http_create_list::error::FileTransportError;
use crate::api::vector_store_search::error::SdkFailure;

// exception_type() maps every attachment failure with model None and the provider the call resolved to
const MODEL: &str = "";

#[derive(Debug, Error)]
pub enum FileCallError {
    #[error("'{provider}' is not a valid LlmProviders")]
    InvalidProvider { provider: String },
    #[error("Vector store file {} is not supported for {provider}", operation.label())]
    Unsupported {
        operation: Operation,
        provider: String,
    },
    #[error("the {field} field must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
    // the Json-to-FileParams bridge; a failure here is a local ValueError, not a provider answer
    #[error("the attachment parameters are not valid")]
    Params(#[source] serde_json::Error),
    #[error("the request timeout of {seconds} seconds is not a valid duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    // B2480's file service, whose provider errors carry the status exception_type() maps by
    #[error(transparent)]
    Service(#[from] FileTransportError),
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...) as the SDK raises it
#[must_use]
pub fn failure(error: &FileCallError, provider: &str, redactor: &Redactor) -> SdkFailure {
    let (message, status, synthesized, exception_type) = match error {
        FileCallError::Service(FileTransportError::Provider(source)) => (
            source.message.clone(),
            Some(source.status_code),
            source.status_code_is_synthesized,
            String::new(),
        ),
        FileCallError::InvalidProvider { .. }
        | FileCallError::Unsupported { .. }
        | FileCallError::Field { .. }
        | FileCallError::Params(_)
        | FileCallError::Timeout { .. }
        | FileCallError::Logging(_)
        | FileCallError::Service(_) => (error.to_string(), None, false, String::from("ValueError")),
    };
    let projected = dispatch(
        &DispatchInput::Raw(Box::new(RawException {
            model: MODEL.to_owned(),
            custom_llm_provider: provider.to_owned(),
            error_str: message.clone(),
            status_code: status,
            status_code_is_synthesized: synthesized,
            message: Some(message.clone()),
            body: None,
            exception_type,
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

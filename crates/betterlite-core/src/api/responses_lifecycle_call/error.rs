use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_error::error::class_message;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::assistant_records::error::class_name;
use crate::api::response_id_traversal::response_id::ResponseRewriteError;
use crate::api::responses_read_http::error::ReadError;

#[derive(Debug, Error)]
pub enum LifecycleError {
    #[error("custom_llm_provider is required but passed as None")]
    MissingProvider,
    #[error("GET responses is not supported for {provider}")]
    UnsupportedRetrieve { provider: String },
    #[error("list_input_items is not supported for {provider}")]
    UnsupportedInputItems { provider: String },
    #[error(transparent)]
    Read(Box<ReadError>),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Rewrite(#[from] ResponseRewriteError),
    #[error("the read answered input items where a response was expected")]
    UnexpectedInputItems,
    #[error("the read answered a response where input items were expected")]
    UnexpectedResponse,
}

// the SDK exception a caller observes: the class the pinned SDK raises and the flat fields its APIError carries
#[derive(Debug)]
pub struct LifecycleFailure {
    pub error: LifecycleError,
    pub class: &'static str,
    pub status: u16,
    pub kind: WireField,
    pub param: WireField,
    pub code: WireField,
    pub message: String,
}

impl LifecycleFailure {
    #[must_use]
    pub fn new(error: LifecycleError, provider: Option<&str>, redactor: &Redactor) -> Self {
        let raw = raw_exception(&error, provider.unwrap_or_default());
        let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
        let metadata = class.metadata();
        Self {
            class: class_name(&class),
            status: metadata.status_code,
            kind: metadata.error_type,
            param: metadata.param,
            code: metadata
                .openai_code
                .map_or(WireField::Null, WireField::Text),
            message: class_message(&class),
            error,
        }
    }
}

// exception_type()'s original_exception: the read handler's BaseLLMException, or the locally raised ValueError
fn raw_exception(error: &LifecycleError, provider: &str) -> RawException {
    if let LifecycleError::Read(read) = error
        && let ReadError::Provider(provider_error) = &**read
    {
        return RawException {
            model: String::new(),
            custom_llm_provider: provider.to_owned(),
            error_str: provider_error.message.clone(),
            status_code: Some(provider_error.status_code),
            status_code_is_synthesized: provider_error.status_code_is_synthesized,
            message: Some(provider_error.message.clone()),
            body: None,
            exception_type: String::from("BaseLLMException"),
        };
    }
    RawException {
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str: error.to_string(),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: String::from("ValueError"),
    }
}

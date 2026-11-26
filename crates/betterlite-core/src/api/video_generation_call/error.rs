use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::video_remix_call::shared::MockDecodeError;
use crate::api::video_selection::error::VideoSelectionError;
use crate::api::videos_http_transport::error::VideoTransportError;
use crate::api::videos_http_transport::video_config::VideoHttpConfig;

// exception_type()'s name for a failure raised outside a provider answer
const VALUE_ERROR: &str = "ValueError";
// get_llm_provider catches its own failures and raises this class with the original model appended
const BAD_REQUEST: &str = "BadRequestError";

#[derive(Debug, Error)]
pub enum VideoGenerationCallError {
    // LiteLLM's ValueError when the selector answers none
    #[error("video generation is not supported for {provider}")]
    Unsupported { provider: String },
    // the provider has a config in LiteLLM, but no adapter for it is installed here yet
    #[error("no video config is installed for '{provider}'")]
    ConfigMissing { provider: String },
    // B3332's own failure, whose message is kept as it comes
    #[error(transparent)]
    Selection(#[from] VideoSelectionError),
    // litellm.LlmProviders(custom_llm_provider) refused the name
    #[error("'{provider}' is not a valid LlmProviders")]
    UnknownProvider {
        provider: String,
        #[source]
        source: strum::ParseError,
    },
    // get_llm_provider refused the model, its own text intact
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    // a mock_response that is not JSON or not a video object
    #[error(transparent)]
    Mock(#[from] MockDecodeError),
    #[error("{field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the request timeout of {seconds} seconds is not a usable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    // the handler's failure as exception_type reads it: the provider's status, or the local text
    #[error("{message}")]
    Transport {
        message: String,
        status_code: Option<u16>,
        status_code_is_synthesized: bool,
    },
    #[error("the blocking video generation call waited on a handler that never answered")]
    Suspended,
}

// the error and the class exception_type gave it, held together so no caller maps it twice
#[derive(Debug, Error)]
#[error("{error}")]
pub struct VideoGenerationFailure {
    #[source]
    pub error: VideoGenerationCallError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model, custom_llm_provider, original_exception) for the failures the call raises itself
#[must_use]
pub fn failure(
    error: VideoGenerationCallError,
    model: Option<&str>,
    provider: Option<&str>,
    redactor: &Redactor,
) -> VideoGenerationFailure {
    let message = error.to_string();
    let provider_name = provider.unwrap_or_default();
    let class = match &error {
        // docs/compatibility.md: a value the declared type cannot hold is a 400 that names the field
        VideoGenerationCallError::FieldType { field, expected } => {
            let context = ErrorContext {
                message: format!("{field} must be {expected}"),
                model: model.map(str::to_owned),
                provider: Some(provider_name.to_owned()),
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: Some((*field).to_owned()),
                body_code: None,
            };
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
        }
        VideoGenerationCallError::Resolution(_) => {
            resolution_class(&message, model, provider_name, redactor)
        }
        // the provider or model is the caller's input and the call can never succeed, so these are 400s alike
        VideoGenerationCallError::Unsupported { .. }
        | VideoGenerationCallError::ConfigMissing { .. }
        | VideoGenerationCallError::Selection(_)
        | VideoGenerationCallError::UnknownProvider { .. } => {
            provider_class(&message, model, provider_name)
        }
        // the local text exception_type gives these: a ValueError with no status
        VideoGenerationCallError::Mock(_)
        | VideoGenerationCallError::Timeout { .. }
        | VideoGenerationCallError::Logging(_)
        | VideoGenerationCallError::Suspended => dispatch(
            &DispatchInput::Raw(Box::new(raw_exception(
                &message,
                model,
                provider_name,
                VALUE_ERROR,
            ))),
            redactor,
        ),
        VideoGenerationCallError::Transport {
            status_code,
            status_code_is_synthesized,
            ..
        } => transport_class(
            &message,
            model,
            provider_name,
            (*status_code, *status_code_is_synthesized),
            redactor,
        ),
    };
    VideoGenerationFailure { error, class }
}

// get_llm_provider wraps its failures: the original model is appended and the class is a 400
fn resolution_class(
    message: &str,
    model: Option<&str>,
    provider: &str,
    redactor: &Redactor,
) -> ErrorClass {
    let text = format!(
        "GetLLMProvider Exception - {message}\n\noriginal model: {}",
        model.unwrap_or("None")
    );
    let mut raw = raw_exception(&text, model, provider, BAD_REQUEST);
    raw.status_code = Some(400);
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

fn provider_class(message: &str, model: Option<&str>, provider: &str) -> ErrorClass {
    ErrorClass::BadRequest(Box::new(ResponseDetails::for_provider(
        message.to_owned(),
        model.map(str::to_owned),
        provider,
    )))
}

// the handler's own class: the provider's status, or the text exception_type gives the absence of one
fn transport_class(
    message: &str,
    model: Option<&str>,
    provider: &str,
    status: (Option<u16>, bool),
    redactor: &Redactor,
) -> ErrorClass {
    let mut raw = raw_exception(message, model, provider, "");
    raw.status_code = status.0;
    raw.status_code_is_synthesized = status.1;
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

// what the B3333 handler's failure carries into exception_type: the provider's own status, or the local text
#[must_use]
pub fn handler_error<Config>(
    error: &VideoTransportError<Config::Error>,
    config: &Config,
) -> VideoGenerationCallError
where
    Config: VideoHttpConfig,
{
    match error {
        VideoTransportError::Provider(carried) => VideoGenerationCallError::Transport {
            message: carried.message.clone(),
            status_code: Some(carried.status_code),
            status_code_is_synthesized: carried.status_code_is_synthesized,
        },
        VideoTransportError::Config(source) => {
            let facts = config.error_facts(source);
            VideoGenerationCallError::Transport {
                message: facts.message,
                status_code: facts.status_code,
                status_code_is_synthesized: false,
            }
        }
        VideoTransportError::Client(_) | VideoTransportError::Logging(_) => {
            VideoGenerationCallError::Transport {
                message: error.to_string(),
                status_code: None,
                status_code_is_synthesized: false,
            }
        }
        VideoTransportError::Field(field) => VideoGenerationCallError::FieldType {
            field: field.field,
            expected: field.expected,
        },
    }
}

fn raw_exception(
    message: &str,
    model: Option<&str>,
    provider: &str,
    exception_type: &str,
) -> RawException {
    RawException {
        model: model.unwrap_or_default().to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.to_owned(),
        status_code: None,
        status_code_is_synthesized: false,
        message: Some(message.to_owned()),
        body: None,
        exception_type: exception_type.to_owned(),
    }
}

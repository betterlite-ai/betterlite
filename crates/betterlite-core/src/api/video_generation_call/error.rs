use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
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
    let details = ResponseDetails::for_provider(
        error.to_string(),
        model.map(str::to_owned),
        provider.unwrap_or_default(),
    );
    let class = failure_class(&error, details, redactor);
    VideoGenerationFailure { error, class }
}

fn failure_class(
    error: &VideoGenerationCallError,
    mut details: ResponseDetails<400>,
    redactor: &Redactor,
) -> ErrorClass {
    let metadata = match error {
        VideoGenerationCallError::FieldType { field, .. } => {
            details.body_param = Some((*field).to_owned());
            return ErrorClass::BadRequest(Box::new(details));
        }
        VideoGenerationCallError::Resolution(_) => {
            details.message = format!(
                "GetLLMProvider Exception - {}\n\noriginal model: {}",
                details.message,
                details.model.as_deref().unwrap_or("None")
            );
            (BAD_REQUEST, Some(400), false)
        }
        VideoGenerationCallError::Unsupported { .. }
        | VideoGenerationCallError::ConfigMissing { .. }
        | VideoGenerationCallError::Selection(_)
        | VideoGenerationCallError::UnknownProvider { .. } => {
            return ErrorClass::BadRequest(Box::new(details));
        }
        VideoGenerationCallError::Mock(_)
        | VideoGenerationCallError::Timeout { .. }
        | VideoGenerationCallError::Logging(_)
        | VideoGenerationCallError::Suspended => (VALUE_ERROR, None, false),
        VideoGenerationCallError::Transport {
            status_code,
            status_code_is_synthesized,
            ..
        } => ("", *status_code, *status_code_is_synthesized),
    };
    dispatch_exception(details, metadata, redactor)
}

fn dispatch_exception(
    details: ResponseDetails<400>,
    metadata: (&str, Option<u16>, bool),
    redactor: &Redactor,
) -> ErrorClass {
    let (exception_type, status_code, status_code_is_synthesized) = metadata;
    let raw = RawException {
        model: details.model.unwrap_or_default(),
        custom_llm_provider: details.provider.unwrap_or_default(),
        error_str: details.message.clone(),
        status_code,
        status_code_is_synthesized,
        message: Some(details.message),
        body: None,
        exception_type: exception_type.to_owned(),
    };
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
        VideoTransportError::Config(source) => config_error(source, config),
        VideoTransportError::Client(_) | VideoTransportError::Logging(_) => {
            local_error::<Config>(error)
        }
        VideoTransportError::Field(field) => VideoGenerationCallError::FieldType {
            field: field.field,
            expected: field.expected,
        },
    }
}

fn config_error<Config>(source: &Config::Error, config: &Config) -> VideoGenerationCallError
where
    Config: VideoHttpConfig,
{
    let facts = config.error_facts(source);
    VideoGenerationCallError::Transport {
        message: facts.message,
        status_code: facts.status_code,
        status_code_is_synthesized: false,
    }
}

fn local_error<Config>(error: &VideoTransportError<Config::Error>) -> VideoGenerationCallError
where
    Config: VideoHttpConfig,
{
    VideoGenerationCallError::Transport {
        message: error.to_string(),
        status_code: None,
        status_code_is_synthesized: false,
    }
}

use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::speech::error::SpeechError;
use betterlite_redact::secret::Redactor;
use strum::VariantArray as _;
use thiserror::Error;

// speech()'s own failures before litellm's exception mapping runs
#[derive(Debug, Error)]
pub enum SpeechCallError {
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    // the litellm.BadRequestError speech raises for a refused voice or a missing config
    #[error("{message}")]
    BadRequest { message: String },
    // the bare Exception at the end of speech, whose text names the provider list
    #[error("{message}")]
    Unmapped { provider: String, message: String },
    // an SDK or provider status failure, keeping the fields B011 maps back to its exception class
    #[error("{message}")]
    Provider {
        status: u16,
        message: String,
        error_type: Option<String>,
        param: Option<String>,
        code: Option<String>,
    },
    // a Python exception the SDK raises outside its status classes: its class carries, its wire fields do not
    #[error("{message}")]
    Sdk {
        class: &'static str,
        message: String,
    },
    #[error("{message}")]
    Internal { message: String },
}

// what a caller of speech() catches: a mapped litellm exception, or a plain Python exception
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechException {
    pub failure: SpeechFailure,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechFailure {
    Mapped(ErrorClass),
    Class { class: &'static str },
}

impl From<SpeechError> for SpeechCallError {
    fn from(error: SpeechError) -> Self {
        match error {
            SpeechError::Status(status) => Self::Provider {
                status: status.status,
                message: status.message,
                error_type: status.error_type,
                param: status.param,
                code: status.code,
            },
            error @ (SpeechError::MissingApiKey
            | SpeechError::BaseUrl { .. }
            | SpeechError::Header { .. }
            | SpeechError::Encode { .. }
            | SpeechError::ClientKind { .. }
            | SpeechError::Transport { .. }
            | SpeechError::Logging { .. }
            | SpeechError::Body { .. }
            | SpeechError::TooLarge) => sdk(&error),
        }
    }
}

// the class the OpenAI SDK raises for each failure B514 can report
fn sdk(error: &SpeechError) -> SpeechCallError {
    let class = match error {
        SpeechError::MissingApiKey
        | SpeechError::BaseUrl { .. }
        | SpeechError::Header { .. }
        | SpeechError::Encode { .. }
        | SpeechError::ClientKind { .. } => "OpenAIError",
        SpeechError::Transport { .. } => "APIConnectionError",
        SpeechError::Logging { .. }
        | SpeechError::Status(_)
        | SpeechError::Body { .. }
        | SpeechError::TooLarge => "Exception",
    };
    SpeechCallError::Sdk {
        class,
        message: error.to_string(),
    }
}

// the message the unmapped branch ends with; betterlite lists its own provider slugs
#[must_use]
pub fn unmapped_message(provider: &str) -> String {
    let providers: Vec<String> = ProviderId::VARIANTS
        .iter()
        .map(ToString::to_string)
        .collect();
    format!("Unable to map the custom llm provider={provider} to a known provider={providers:?}.")
}

// litellm's exception_type over the failure: status, provider and body pick the class
#[must_use]
pub fn exception(
    error: &SpeechCallError,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> SpeechException {
    match error {
        SpeechCallError::Resolution(source) => {
            mapped(source.to_string(), Some(400_u16), model, provider, redactor)
        }
        SpeechCallError::BadRequest { message } => {
            mapped(message.clone(), Some(400_u16), model, provider, redactor)
        }
        SpeechCallError::Provider {
            status,
            message,
            error_type,
            param,
            code,
        } => {
            let body = RawExceptionBody {
                kind: error_type.clone(),
                param: param.clone(),
                code: code.clone(),
                message: Some(message.clone()),
                error: None,
            };
            let raw = raw_exception(message, Some(*status), Some(body), model, provider);
            let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
            SpeechException {
                failure: SpeechFailure::Mapped(class),
                message: message.clone(),
            }
        }
        SpeechCallError::Sdk { class, message } => SpeechException {
            failure: SpeechFailure::Class { class },
            message: message.clone(),
        },
        SpeechCallError::Unmapped { message, .. } | SpeechCallError::Internal { message } => {
            SpeechException {
                failure: SpeechFailure::Class { class: "Exception" },
                message: message.clone(),
            }
        }
    }
}

fn mapped(
    message: String,
    status: Option<u16>,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> SpeechException {
    let raw = raw_exception(&message, status, None, model, provider);
    let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
    SpeechException {
        failure: SpeechFailure::Mapped(class),
        message,
    }
}

fn raw_exception(
    message: &str,
    status: Option<u16>,
    body: Option<RawExceptionBody>,
    model: &str,
    provider: &str,
) -> RawException {
    RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.to_owned(),
        status_code: status,
        status_code_is_synthesized: false,
        message: Some(message.to_owned()),
        body,
        exception_type: String::new(),
    }
}

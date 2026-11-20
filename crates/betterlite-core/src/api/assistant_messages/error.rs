use std::str::from_utf8;
use std::time::TryFromFloatSecsError;

use betterlite_azure::azure_message_attachments::error::MessagesError;
use betterlite_error::dispatch::{
    DispatchInput, RawException, RawExceptionBody, RawExceptionBodyDetail, dispatch,
};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_openai::threads::error::ThreadsError;
use betterlite_prompts::arguments::json::Json;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::resource_sdk_settings::timeout::TimeoutError;

use super::options::OptionError;

#[derive(Debug, Error)]
pub enum SetupError {
    #[error(transparent)]
    Options(#[from] OptionError),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("the selected message is not a valid SDK message: {source}")]
    Data {
        #[source]
        source: serde_json::Error,
    },
    #[error("the timeout {seconds} seconds is not a duration")]
    Deadline {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("LiteLLM doesn't support {provider} for '{operation}'. Only 'openai' is supported.")]
    UnsupportedProvider {
        provider: String,
        operation: &'static str,
    },
}

#[derive(Debug, Error)]
pub enum MessageError {
    #[error(transparent)]
    Setup(#[from] SetupError),
    #[error(transparent)]
    OpenAi(#[from] ThreadsError),
    #[error(transparent)]
    Azure(#[from] MessagesError),
    #[error("the provider answered the {operation} call with the wrong SDK object")]
    UnexpectedReply { operation: &'static str },
}

impl MessageError {
    // the status and body a provider answer carried, which the raw SDK exception reports as-is
    #[must_use]
    pub const fn provider_answer(&self) -> Option<(u16, &[u8])> {
        match self {
            Self::OpenAi(ThreadsError::Status { status, body, .. })
            | Self::Azure(MessagesError::Threads(ThreadsError::Status { status, body, .. })) => {
                Some((*status, body.as_slice()))
            }
            Self::Setup(_) | Self::OpenAi(_) | Self::Azure(_) | Self::UnexpectedReply { .. } => {
                None
            }
        }
    }
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct MessageFailure {
    #[source]
    pub error: MessageError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(error: MessageError, provider: &str, redactor: &Redactor) -> MessageFailure {
    let class = match &error {
        MessageError::Setup(setup) => setup_class(setup, provider),
        MessageError::OpenAi(_) | MessageError::Azure(_) => dispatch(
            &DispatchInput::Raw(Box::new(exception(&error, provider))),
            redactor,
        ),
        MessageError::UnexpectedReply { .. } => {
            ErrorClass::Unclassified(Box::new(context(error.to_string(), provider)))
        }
    };
    MessageFailure { error, class }
}

// the selector's own classes never reach the provider, so they map straight from its error
fn setup_class(error: &SetupError, provider: &str) -> ErrorClass {
    match error {
        SetupError::Options(OptionError::MalformedAttachment)
        | SetupError::Timeout(_)
        | SetupError::Data { .. }
        | SetupError::Deadline { .. } => {
            ErrorClass::Unclassified(Box::new(context(error.to_string(), provider)))
        }
        SetupError::Options(options) => {
            ErrorClass::UnsupportedParams(Box::new(context(options.litellm_message(), provider)))
        }
        SetupError::UnsupportedProvider { .. } => {
            let details = ErrorContext {
                model: Some("n/a".to_owned()),
                ..context(error.to_string(), provider)
            };
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(details, None)))
        }
    }
}

fn context(message: String, provider: &str) -> ErrorContext {
    ErrorContext {
        message,
        model: None,
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

// the openai SDK's APIStatusError: str(e) is "Error code: {status} - {body}" and e.body is the error object
fn exception(error: &MessageError, provider: &str) -> RawException {
    let answer = error.provider_answer();
    let status = answer.map(|(status, _body)| status);
    let body = answer.map(|(_status, body)| body);
    let error_str = status.map_or_else(
        || error.to_string(),
        |status_code| {
            format!(
                "Error code: {status_code} - {}",
                body.map_or("", |bytes| from_utf8(bytes).unwrap_or(""))
            )
        },
    );
    RawException {
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str,
        status_code: status,
        status_code_is_synthesized: false,
        message: None,
        body: body.and_then(flattened_body),
        exception_type: String::new(),
    }
}

// _make_status_error hands APIError the object under "error" when the parsed body is a mapping (openai/_client.py#L687)
fn flattened_body(bytes: &[u8]) -> Option<RawExceptionBody> {
    let Json::Object(mut fields) = serde_json::from_slice::<Json>(bytes).ok()? else {
        return None;
    };
    let flat = match fields.shift_remove("error") {
        Some(Json::Object(inner)) => inner,
        // a non-mapping body keeps no fields, so the whole object stands in for it
        Some(_) => return Some(RawExceptionBody::default()),
        None => fields,
    };
    let error = match flat.get("error") {
        Some(Json::Object(inner)) => Some(RawExceptionBodyDetail {
            kind: None,
            param: None,
            code: None,
            message: inner
                .get("message")
                .and_then(Json::as_str)
                .map(str::to_owned),
            inner_error: None,
            innererror: None,
        }),
        Some(Json::Text(_) | Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_))
        | None => None,
    };
    let text = |name: &str| flat.get(name).and_then(Json::as_str).map(str::to_owned);
    Some(RawExceptionBody {
        kind: text("type"),
        param: text("param"),
        code: text("code"),
        message: text("message"),
        error,
    })
}

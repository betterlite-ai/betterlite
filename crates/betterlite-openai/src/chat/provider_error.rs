use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_redact::secret::Redactor;
use serde_json::Value;

use crate::chat::execution::ChatError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFailure {
    pub status: u16,
    pub error_type: String,
    pub param: Option<String>,
}

#[derive(Clone, Copy)]
pub enum ErrorBodyKind {
    Structured,
    MessageOnly,
}

impl ChatError {
    // the client status, type and param of any variant; Provider runs the same dispatch the gateway bodies use
    #[must_use]
    pub fn failure(&self, redactor: &Redactor) -> ProviderFailure {
        let status = match self {
            Self::UnknownModel { .. } | Self::Request { .. } => 400,
            Self::UnsupportedProvider { .. } | Self::Streaming => 501,
            Self::MissingApiKey => 401,
            Self::Timeout { .. } => 408,
            Self::Provider { response } => {
                return provider_failure(response, redactor, ErrorBodyKind::Structured);
            }
            Self::Http { .. } | Self::Encode { .. } | Self::Decode { .. } | Self::TooLarge => 500,
        };
        ProviderFailure {
            status,
            error_type: fallback_type(status).to_owned(),
            param: None,
        }
    }
}

#[must_use]
pub fn provider_failure(
    response: &UpstreamResponse,
    redactor: &Redactor,
    error_body: ErrorBodyKind,
) -> ProviderFailure {
    rendered(&dispatch(
        &DispatchInput::Raw(Box::new(raw_exception(response, error_body))),
        redactor,
    ))
}

// LiteLLM's openai_error_type: the exception's own type unless it is the string "None", else the client type from the status
#[must_use]
pub fn rendered(class: &ErrorClass) -> ProviderFailure {
    let metadata = class.metadata();
    // _handle_llm_api_exception keeps a provider status only in 400..=599 and answers 500 for every other one
    let status = if (400..=599).contains(&metadata.status_code) {
        metadata.status_code
    } else {
        500
    };
    let error_type = match metadata.error_type {
        WireField::Text(text) if text != "None" => text,
        WireField::Text(_) | WireField::Missing(_) | WireField::Null => {
            fallback_type(status).to_owned()
        }
    };
    let param = match metadata.param {
        WireField::Text(text) if text != "None" => Some(text),
        WireField::Text(_) | WireField::Missing(_) | WireField::Null => None,
    };
    ProviderFailure {
        status,
        error_type,
        param,
    }
}

const fn fallback_type(status: u16) -> &'static str {
    match status {
        401 => "authentication_error",
        403 => "permission_error",
        429 => "rate_limit_error",
        0..500 => "invalid_request_error",
        _ => "internal_server_error",
    }
}

// the openai SDK's APIStatusError: str(e) is "Error code: {status} - {body}" and e.body is the error object, or the whole body without one
fn raw_exception(response: &UpstreamResponse, error_body: ErrorBodyKind) -> RawException {
    let body = match error_body {
        // LiteLLM's embedding wrapper preserves error text but omits the SDK's structured body.
        ErrorBodyKind::MessageOnly => None,
        ErrorBodyKind::Structured => sdk_body(&response.body),
    };
    RawException {
        model: String::new(),
        custom_llm_provider: "openai".to_owned(),
        error_str: format!(
            "Error code: {} - {}",
            response.status,
            str::from_utf8(&response.body).unwrap_or_default()
        ),
        status_code: Some(response.status),
        status_code_is_synthesized: false,
        message: None,
        body,
        exception_type: String::new(),
    }
}

fn sdk_body(bytes: &[u8]) -> Option<RawExceptionBody> {
    let data = serde_json::from_slice::<Value>(bytes).ok()?;
    let error = data.get("error").unwrap_or(&data).as_object()?;
    let text = |name: &str| error.get(name).and_then(Value::as_str).map(str::to_owned);
    Some(RawExceptionBody {
        kind: text("type"),
        param: text("param"),
        code: text("code"),
        message: text("message"),
        error: None,
    })
}

mod tests;

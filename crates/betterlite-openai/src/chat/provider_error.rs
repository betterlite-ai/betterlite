use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_error::openai::body::OpenaiErrorBody;
use betterlite_error::openai::{OpenaiExceptionInput, map_openai_exception};
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
    // A failure rendered without a Redactor, for callers that read the status, type and param and never the message.
    #[must_use]
    pub fn failure(&self) -> ProviderFailure {
        let status = match self {
            Self::UnknownModel { .. } | Self::Request { .. } => 400,
            Self::UnsupportedProvider { .. } | Self::Streaming => 501,
            Self::MissingApiKey => 401,
            Self::Timeout { .. } => 408,
            Self::Provider { response } => {
                return provider_failure(response, None, ErrorBodyKind::Structured);
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

// with a Redactor the error runs LiteLLM's full dispatch, without one it maps straight through map_openai_exception
#[must_use]
pub fn provider_failure(
    response: &UpstreamResponse,
    redactor: Option<&Redactor>,
    error_body: ErrorBodyKind,
) -> ProviderFailure {
    let raw = raw_exception(response, error_body);
    let class = if let Some(active) = redactor {
        dispatch(&DispatchInput::Raw(Box::new(raw)), active)
    } else {
        let input = OpenaiExceptionInput {
            model: raw.model,
            custom_llm_provider: raw.custom_llm_provider,
            error_str: raw.error_str,
            status_code: raw.status_code,
            status_code_is_synthesized: raw.status_code_is_synthesized,
            message: raw.message,
            body: raw.body.map(|body| OpenaiErrorBody {
                kind: body.kind,
                param: body.param,
                code: body.code,
                message: body.message,
                error: None,
            }),
        };
        map_openai_exception(&input)
    };
    rendered(&class)
}

// LiteLLM's openai_error_type: the exception's own type unless it is the string "None", else the client type from the status
#[must_use]
pub fn rendered(class: &ErrorClass) -> ProviderFailure {
    let metadata = class.metadata();
    let error_type = match metadata.error_type {
        WireField::Text(text) if text != "None" => text,
        WireField::Text(_) | WireField::Missing(_) | WireField::Null => {
            fallback_type(metadata.status_code).to_owned()
        }
    };
    let param = match metadata.param {
        WireField::Text(text) if text != "None" => Some(text),
        WireField::Text(_) | WireField::Missing(_) | WireField::Null => None,
    };
    ProviderFailure {
        status: metadata.status_code,
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

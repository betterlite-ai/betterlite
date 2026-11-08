use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct HttpException {
    pub status: u16,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ProxyException {
    pub message: String,
    pub error_type: String,
    pub param: Option<String>,
    pub code: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("sensitive data detected in the request, routing to {route_to_model}")]
pub struct SensitiveRouteError {
    pub session_id: String,
    pub route_to_model: String,
    pub guardrail_name: Option<String>,
    pub sticky_session_routing: bool,
    pub detection_info: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ModifiedResponseError {
    pub message: String,
    pub model: String,
    pub guardrail_name: Option<String>,
    pub detection_info: BTreeMap<String, JsonValue>,
    pub original_response: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProxyHookError {
    #[error("{detail}")]
    Http { status: u16, detail: String },
    #[error("{0}")]
    Proxy(Box<ProxyException>),
    #[error("{message}")]
    GuardrailRaised {
        message: String,
        guardrail_name: Option<String>,
        status: u16,
        blocked_content: bool,
    },
    #[error("{0}")]
    SensitiveDataRoute(Box<SensitiveRouteError>),
    #[error("{0}")]
    ModifyResponse(Box<ModifiedResponseError>),
    #[error("{message}")]
    RejectedRequest { message: String, model: String },
    #[error("{0}")]
    Other(String),
}

impl From<ProxyException> for ProxyHookError {
    fn from(exception: ProxyException) -> Self {
        Self::Proxy(Box::new(exception))
    }
}

impl From<HttpException> for ProxyHookError {
    fn from(exception: HttpException) -> Self {
        Self::Http {
            status: exception.status,
            detail: exception.detail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuardrailGateError {
    #[error("a tag-based event hook cannot be evaluated outside the enterprise tag logic")]
    TagMode,
    #[error("the request's guardrails value is not a list")]
    RequestedNotList,
}

#[must_use]
pub fn is_client_error(error: &ProxyHookError) -> bool {
    match error {
        ProxyHookError::Http { status, .. } => *status < 500,
        ProxyHookError::Proxy(exception) => !code_is_server_range(&exception.code),
        ProxyHookError::GuardrailRaised { .. }
        | ProxyHookError::SensitiveDataRoute(_)
        | ProxyHookError::ModifyResponse(_)
        | ProxyHookError::RejectedRequest { .. }
        | ProxyHookError::Other(_) => false,
    }
}

// Python's isdigit on an int() that may overflow: only a numeric code of 500 or more is server-side
fn code_is_server_range(code: &str) -> bool {
    if let Ok(number) = code.parse::<u32>() {
        return number >= 500;
    }
    !code.is_empty() && code.as_bytes().iter().all(u8::is_ascii_digit)
}

#[must_use]
pub const fn changes_request_flow(error: &ProxyHookError) -> bool {
    match error {
        ProxyHookError::SensitiveDataRoute(_) | ProxyHookError::ModifyResponse(_) => true,
        ProxyHookError::Http { .. }
        | ProxyHookError::Proxy(_)
        | ProxyHookError::GuardrailRaised { .. }
        | ProxyHookError::RejectedRequest { .. }
        | ProxyHookError::Other(_) => false,
    }
}

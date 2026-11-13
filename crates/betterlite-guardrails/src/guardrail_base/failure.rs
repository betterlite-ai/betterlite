use std::collections::BTreeMap;

use indexmap::IndexMap;
use thiserror::Error;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

// LiteLLM's ModifyResponseException
#[derive(Debug, Clone, PartialEq)]
pub struct ModifyResponseFailure {
    pub message: String,
    pub model: String,
    pub request_data: BTreeMap<String, GuardrailValue>,
    pub guardrail_name: Option<String>,
    pub detection_info: BTreeMap<String, GuardrailValue>,
    pub original_response: Option<GuardrailValue>,
}

// LiteLLM's SensitiveDataRouteException
#[derive(Debug, Clone, PartialEq)]
pub struct SensitiveDataRouteFailure {
    pub route_to_model: String,
    pub session_id: String,
    pub guardrail_name: Option<String>,
    pub detection_info: BTreeMap<String, GuardrailValue>,
    pub message: Option<String>,
    pub sticky_session_routing: bool,
}

// LiteLLM prints a missing guardrail name as None
#[derive(Debug, Clone, PartialEq, Error)]
pub enum GuardrailFailure {
    #[error("{}", .0.message)]
    ModifyResponse(Box<ModifyResponseFailure>),
    #[error("{}", raised_text(.guardrail_name.as_deref(), .message, *.wrap_default_message))]
    GuardrailRaised {
        guardrail_name: Option<String>,
        message: String,
        wrap_default_message: bool,
        status_code: u16,
        blocked_content: bool,
    },
    #[error(
        "Blocked entity detected: {entity_type} by Guardrail: {}. This entity is not allowed to be used in this request.",
        .guardrail_name.as_deref().unwrap_or("None")
    )]
    BlockedPii {
        entity_type: String,
        guardrail_name: Option<String>,
        status_code: u16,
    },
    #[error("{}", route_text(.0))]
    SensitiveDataRoute(Box<SensitiveDataRouteFailure>),
    #[error("{detail}")]
    Http { status: u16, detail: String },
    #[error("{}", object_text(.detail))]
    HttpObject {
        status: u16,
        detail: IndexMap<String, GuardrailValue>,
    },
    #[error("{0}")]
    Other(String),
}

fn raised_text(guardrail_name: Option<&str>, message: &str, wrap_default_message: bool) -> String {
    if wrap_default_message {
        let name = guardrail_name.unwrap_or("None");
        return format!("Guardrail raised an exception, Guardrail: {name}, Message: {message}");
    }
    message.to_owned()
}

fn route_text(failure: &SensitiveDataRouteFailure) -> String {
    if let Some(message) = &failure.message {
        return message.clone();
    }
    let name = failure.guardrail_name.as_deref().unwrap_or("None");
    format!(
        "Sensitive data detected by {name}. Routing to model: {}",
        failure.route_to_model
    )
}

fn object_text(detail: &IndexMap<String, GuardrailValue>) -> String {
    serde_json::to_string(detail)
        .unwrap_or_else(|error| format!("detail could not be written as JSON: {error}"))
}

impl GuardrailFailure {
    // LiteLLM's is_guardrail_intervention: 401, 408 and 429 are provider trouble, not a block
    #[must_use]
    pub const fn is_intervention(&self) -> bool {
        match self {
            Self::ModifyResponse(_)
            | Self::GuardrailRaised { .. }
            | Self::BlockedPii { .. }
            | Self::SensitiveDataRoute(_) => true,
            Self::Http { status, .. } | Self::HttpObject { status, .. } => {
                matches!(status, 400 | 403 | 422)
            }
            Self::Other(_) => false,
        }
    }
}

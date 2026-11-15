use indexmap::IndexMap;
use reqwest::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_initializers::error::InitializerError;
use crate::hiddenlayer::hiddenlayer_authentication::error::HiddenlayerError;

#[derive(Debug, Error)]
pub enum EvaluationError {
    #[error(transparent)]
    Base(#[from] GuardrailBaseError),
    #[error(transparent)]
    Initializer(#[from] InitializerError),
    #[error(transparent)]
    Auth(#[from] HiddenlayerError),
    #[error("the HiddenLayer evaluation response is not JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the HiddenLayer evaluation response has no usable {field}")]
    Shape { field: &'static str },
    #[error("a HiddenLayer request header name is not a valid header name")]
    InvalidHeaderName {
        #[source]
        source: InvalidHeaderName,
    },
    #[error("a HiddenLayer request header value is not a valid header value")]
    InvalidHeader {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the HiddenLayer evaluation request could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("Violated guardrail policy")]
    Blocked {
        status: u16,
        detail: IndexMap<String, GuardrailValue>,
    },
}

// Both versions refuse a call with this body, v1 after adding its two scan fields
pub fn blocked(entries: Vec<(&str, GuardrailValue)>) -> EvaluationError {
    let detail = entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();
    EvaluationError::Blocked {
        status: 400,
        detail,
    }
}

// A v1 redacted message keeps every field the endpoint returned; a role it left out reads as user
pub fn as_guardrail_message(message: &GuardrailValue) -> Result<GuardrailMessage, EvaluationError> {
    let GuardrailValue::Object(fields) = message else {
        return Err(EvaluationError::Shape {
            field: "modified_data.messages",
        });
    };
    let mut extra = fields.clone();
    let role = match extra.remove("role") {
        Some(GuardrailValue::Text(role)) => role,
        _ => "user".to_owned(),
    };
    Ok(GuardrailMessage { role, extra })
}

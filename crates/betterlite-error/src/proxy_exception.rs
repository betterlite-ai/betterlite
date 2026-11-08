#![expect(
    clippy::disallowed_types,
    clippy::option_if_let_else,
    clippy::ref_option,
    reason = "LiteLLM's provider_specific_fields is a dict of provider-defined keys, so it stays an opaque JSON object, and serde hands the field to its skip predicate by reference"
)]

use std::collections::BTreeMap;
use std::error::Error;

use serde_json::{Map, Value};

pub mod body;
pub mod model_access_denied;
mod tests;

use self::body::ProxyErrorBody;

const NO_DEPLOYMENT_PHRASES: [&str; 2] = [
    "No healthy deployment available",
    "No deployments available",
];
const TAG_ROUTING_MESSAGE: &str = "Not allowed to access model due to tags configuration";

// LiteLLM accepts an int, a string or None for the code and stores str(code)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ExceptionCode {
    #[default]
    Absent,
    Status(i64),
    Text(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyExceptionInit {
    pub message: String,
    pub error_type: String,
    pub param: Option<String>,
    pub code: ExceptionCode,
    pub headers: BTreeMap<String, String>,
    pub openai_code: Option<String>,
    pub provider_specific_fields: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, derive_more::Display)]
#[display("{}", message)]
pub struct ProxyException {
    pub message: String,
    pub error_type: String,
    pub param: Option<String>,
    pub code: String,
    pub openai_code: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub provider_specific_fields: Option<Map<String, Value>>,
}

impl ProxyException {
    #[must_use]
    pub fn new(init: ProxyExceptionInit) -> Self {
        let ProxyExceptionInit {
            message,
            error_type,
            param,
            code,
            headers,
            openai_code,
            provider_specific_fields,
        } = init;
        let stored_openai_code = match openai_code {
            Some(value) if !value.is_empty() => Some(value),
            _ => openai_code_from(&code),
        };
        let stored_code = if NO_DEPLOYMENT_PHRASES
            .iter()
            .any(|phrase| message.contains(phrase))
        {
            "429".to_owned()
        } else if message.contains(TAG_ROUTING_MESSAGE) {
            "401".to_owned()
        } else {
            match &code {
                ExceptionCode::Absent => "None".to_owned(),
                ExceptionCode::Status(status) => status.to_string(),
                ExceptionCode::Text(text) => text.clone(),
            }
        };
        Self {
            message,
            error_type,
            param,
            code: stored_code,
            openai_code: stored_openai_code,
            headers,
            provider_specific_fields,
        }
    }

    #[must_use]
    pub fn to_body(&self) -> ProxyErrorBody {
        ProxyErrorBody {
            message: self.message.clone(),
            error_type: self.error_type.clone(),
            param: self.param.clone(),
            code: self.code.clone(),
            provider_specific_fields: self.provider_specific_fields.clone(),
        }
    }
}

fn openai_code_from(code: &ExceptionCode) -> Option<String> {
    match code {
        ExceptionCode::Absent => None,
        ExceptionCode::Status(status) => Some(status.to_string()),
        ExceptionCode::Text(text) => Some(text.clone()),
    }
}

impl Error for ProxyException {}

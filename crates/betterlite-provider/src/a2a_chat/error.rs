use std::collections::BTreeMap;

use betterlite_a2a::a2a_text::{RecordOr, Shape};
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Deserialize)]
pub struct ErrorDetail {
    message: Option<Shape>,
}

#[must_use]
pub fn error_message(error: &RecordOr<ErrorDetail>) -> String {
    let detail = match error {
        RecordOr::Record(ErrorDetail {
            message: Some(Shape::Text(text)),
        }) => text.as_str(),
        RecordOr::Record(_) | RecordOr::Other(_) => "Unknown error",
    };
    format!("A2A error: {detail}")
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct A2aError {
    pub status_code: u16,
    pub message: String,
    pub headers: BTreeMap<String, String>,
}

impl From<A2aError> for ProviderError {
    fn from(error: A2aError) -> Self {
        Self::from(ProviderErrorParts {
            status_code: error.status_code,
            message: error.message,
            headers: (!error.headers.is_empty()).then_some(error.headers.into_iter().collect()),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

#[must_use]
pub fn get_error_class(
    message: &str,
    status_code: u16,
    headers: BTreeMap<String, String>,
) -> A2aError {
    A2aError {
        status_code,
        message: message.to_owned(),
        headers,
    }
}

#[derive(Debug, Error)]
pub enum A2aFailure {
    #[error("At least one message is required for A2A completion")]
    MessagesRequired,
    #[error("api_base is required for A2A provider")]
    MissingApiBase,
    #[error("the A2A option {field} has the wrong type")]
    OptionType { field: &'static str },
    #[error("the A2A agent card url is not a string")]
    CardUrlType,
    #[error("the A2A agent header {name} is not a string")]
    HeaderType { name: String },
    #[error("the A2A agent Entra credentials cannot be resolved: {0}")]
    Credentials(#[from] CredentialsError),
}

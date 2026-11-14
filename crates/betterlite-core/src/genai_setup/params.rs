use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;
use thiserror::Error;

// A kwargs field the setup reads with the wrong JSON type: docs/compatibility.md names the field instead of coercing.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{field} must be {expected}")]
pub struct RequestFieldError {
    pub field: &'static str,
    pub expected: &'static str,
}

// The GenericLiteLLMParams fields this setup reads; kwargs carries the rest through untouched.
#[derive(Debug, Clone)]
pub struct GenerateContentLiteLLMParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub max_retries: Option<i64>,
    pub organization: Option<String>,
    pub mock_response: Option<Json>,
}

impl GenerateContentLiteLLMParams {
    pub fn from_kwargs(kwargs: &Object) -> Result<Self, RequestFieldError> {
        Ok(Self {
            api_base: optional_string(kwargs, "api_base")?,
            api_key: optional_string(kwargs, "api_key")?.map(SecretString::from),
            api_version: optional_string(kwargs, "api_version")?,
            custom_llm_provider: optional_string(kwargs, "custom_llm_provider")?,
            max_retries: match kwargs.get("max_retries") {
                None | Some(Json::Null) => None,
                Some(Json::Number(number)) => Some(number.as_i64().ok_or(RequestFieldError {
                    field: "max_retries",
                    expected: "an integer",
                })?),
                Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
                    return Err(RequestFieldError {
                        field: "max_retries",
                        expected: "an integer",
                    });
                }
            },
            organization: optional_string(kwargs, "organization")?,
            mock_response: match kwargs.get("mock_response") {
                None | Some(Json::Null) => None,
                Some(value) => Some(value.clone()),
            },
        })
    }
}

// absent and null both take the field's default; any other type is an error that names the field
pub fn optional_string(
    kwargs: &Object,
    field: &'static str,
) -> Result<Option<String>, RequestFieldError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(RequestFieldError {
                field,
                expected: "a string",
            })
        }
    }
}

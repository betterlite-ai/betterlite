use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OpenAiError {
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
    #[error("response_format has no {key} key")]
    MissingKey { key: &'static str },
    #[error("response_format {key} is not an object")]
    NotSubscriptable { key: &'static str },
    #[error("a $ref value is not a string")]
    RefNotText,
    #[error("ref_template names the field {field}, and only model is supplied")]
    TemplateField { field: String },
    #[error("ref_template has a positional field")]
    TemplatePosition,
    #[error("ref_template has an unmatched brace")]
    TemplateBrace,
    #[error("the existing tools value is not a list")]
    ToolsNotList,
    #[error("thinking budget_tokens is not an integer")]
    BudgetNotInteger,
    #[error("thinking budget_tokens plus the default max tokens overflows")]
    BudgetOverflow,
    #[error("a provider config does not serialize to an object")]
    ConfigNotObject,
    #[error("a provider config does not serialize: {source}")]
    ConfigSerialize {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorRequest {
    pub method: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorResponse {
    pub status_code: u16,
    pub request: ErrorRequest,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderErrorParts {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Vec<(String, String)>>,
    pub request: Option<ErrorRequest>,
    pub response: Option<ErrorResponse>,
    pub body: Option<Map<String, Value>>,
    pub status_code_is_synthesized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderError {
    pub status_code: u16,
    pub status_code_is_synthesized: bool,
    pub message: String,
    pub headers: Option<Vec<(String, String)>>,
    pub request: ErrorRequest,
    pub response: ErrorResponse,
    pub body: Option<Map<String, Value>>,
}

impl From<ProviderErrorParts> for ProviderError {
    fn from(parts: ProviderErrorParts) -> Self {
        let request = parts.request.unwrap_or_else(|| ErrorRequest {
            method: "POST".to_owned(),
            url: "https://docs.litellm.ai/docs".to_owned(),
        });
        let response = parts.response.unwrap_or_else(|| ErrorResponse {
            status_code: parts.status_code,
            request: request.clone(),
        });
        Self {
            status_code: parts.status_code,
            status_code_is_synthesized: parts.status_code_is_synthesized,
            message: parts.message,
            headers: parts.headers,
            request,
            response,
            body: parts.body,
        }
    }
}

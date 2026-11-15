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

// CredentialLiteLLMParams declares vertex_credentials as a path string or a service-account object
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VertexCredentials {
    Path(String),
    ServiceAccount(Object),
}

// The GenericLiteLLMParams fields this setup models; extra keeps the rest the way extra="allow" does, so the handlers still see every kwarg.
#[derive(Debug, Clone)]
pub struct GenerateContentLiteLLMParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub max_retries: Option<i64>,
    pub organization: Option<String>,
    pub mock_response: Option<Json>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<VertexCredentials>,
    pub ssl_verify: Option<Json>,
    pub extra: Object,
}

// the kwargs this struct models, which therefore never land in extra
const MODELED: [&str; 11] = [
    "api_base",
    "api_key",
    "api_version",
    "custom_llm_provider",
    "max_retries",
    "organization",
    "mock_response",
    "vertex_project",
    "vertex_location",
    "vertex_credentials",
    "ssl_verify",
];

impl GenerateContentLiteLLMParams {
    pub fn from_kwargs(kwargs: &Object) -> Result<Self, RequestFieldError> {
        Ok(Self {
            api_base: optional_string(kwargs, "api_base")?,
            api_key: optional_string(kwargs, "api_key")?.map(SecretString::from),
            api_version: optional_string(kwargs, "api_version")?,
            custom_llm_provider: optional_string(kwargs, "custom_llm_provider")?,
            max_retries: optional_integer(kwargs, "max_retries")?,
            organization: optional_string(kwargs, "organization")?,
            mock_response: optional_value(kwargs, "mock_response"),
            vertex_project: optional_string(kwargs, "vertex_project")?,
            vertex_location: optional_string(kwargs, "vertex_location")?,
            vertex_credentials: vertex_credentials(kwargs)?,
            ssl_verify: optional_value(kwargs, "ssl_verify"),
            extra: remaining_kwargs(kwargs),
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
        Some(value) => value
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or(RequestFieldError {
                field,
                expected: "a string",
            }),
    }
}

fn optional_integer(
    kwargs: &Object,
    field: &'static str,
) -> Result<Option<i64>, RequestFieldError> {
    let Some(value) = optional_value(kwargs, field) else {
        return Ok(None);
    };
    if let Json::Number(number) = &value
        && let Some(integer) = number.as_i64()
    {
        return Ok(Some(integer));
    }
    Err(RequestFieldError {
        field,
        expected: "an integer",
    })
}

// absent and null both take the default, and any other value is kept as written
fn optional_value(kwargs: &Object, field: &'static str) -> Option<Json> {
    match kwargs.get(field) {
        None | Some(Json::Null) => None,
        Some(value) => Some(value.clone()),
    }
}

fn vertex_credentials(kwargs: &Object) -> Result<Option<VertexCredentials>, RequestFieldError> {
    let invalid = RequestFieldError {
        field: "vertex_credentials",
        expected: "a string or an object",
    };
    optional_value(kwargs, "vertex_credentials").map_or(Ok(None), |value| match value {
        Json::Text(path) => Ok(Some(VertexCredentials::Path(path))),
        Json::Object(account) => Ok(Some(VertexCredentials::ServiceAccount(account))),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => Err(invalid),
    })
}

// extra="allow": every kwarg the struct does not model stays on the record for the call handlers
fn remaining_kwargs(kwargs: &Object) -> Object {
    kwargs
        .iter()
        .filter(|(name, _)| !MODELED.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

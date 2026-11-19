use std::str::FromStr as _;

use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::{HiddenParams, Json as HiddenJson};
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Number, Value};
use thiserror::Error;

use crate::openai_base_config::{Headers, Object};

mod number;
mod tests;
pub const EDENAI_API_BASE: &str = "https://api.edenai.run/v3";
pub const EDENAI_COST_HEADER: &str = "x-edenai-cost";
pub const MISSING_KEY_MESSAGE: &str = "Missing Eden AI API key: set EDENAI_API_KEY or pass api_key";
pub const PROVIDER: &str = "edenai";
pub const RESPONSE_COST_HEADER: &str = "llm_provider-x-litellm-response-cost";

const ADDITIONAL_HEADERS: &str = "additional_headers";
const AUTHORIZATION: &str = "Authorization";
const CONTENT_TYPE: &str = "Content-Type";
const COST: &str = "cost";
const JSON_CONTENT_TYPE: &str = "application/json";

// LiteLLM's EdenAIException is a BaseLLMException: a status, a message and the reply headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct EdenAiError {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
}

// LiteLLM's get_error_class for every Eden endpoint builds EdenAIException from the reply
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: Option<&Headers>) -> EdenAiError {
    EdenAiError {
        status_code,
        message: message.to_owned(),
        headers: headers.cloned().unwrap_or_default(),
    }
}

// a catalog body LiteLLM's model_validate rejects becomes the proxy's generic 500, with the field named
#[must_use]
pub fn catalog_reply_error(error: CatalogError, headers: &Headers) -> EdenAiError {
    EdenAiError {
        status_code: 500,
        message: error.to_string(),
        headers: headers.clone(),
    }
}

// the caller fills these from EDENAI_API_KEY, EDENAI_API_BASE and litellm.api_key
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EdenAiEnv {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub global_api_key: Option<SecretString>,
}

// LiteLLM raises its AuthenticationError before any request leaves for a call with no key
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{MISSING_KEY_MESSAGE}")]
pub struct MissingApiKey {
    pub model: String,
    pub status_code: u16,
}

impl MissingApiKey {
    #[must_use]
    #[rustfmt::skip]
    pub fn class(&self) -> ErrorClass {
        ErrorClass::Authentication(Box::new(ResponseDetails::from(ErrorContext {
            message: self.to_string(), model: Some(self.model.clone()),
            provider: Some(PROVIDER.to_owned()), max_retries: None, num_retries: None,
            body_type: None, body_param: None, body_code: None,
        })))
    }
}

// explicit base, then the environment value, then the default; an empty value counts as unset
#[must_use]
pub fn resolve_api_base(api_base: Option<&str>, env: &EdenAiEnv) -> String {
    api_base
        .filter(|value| !value.is_empty())
        .or_else(|| env.api_base.as_deref().filter(|value| !value.is_empty()))
        .map_or_else(|| EDENAI_API_BASE.to_owned(), str::to_owned)
}

// explicit key, then EDENAI_API_KEY; litellm.api_key is added on top by require_api_key
#[must_use]
pub fn resolve_api_key(api_key: Option<&SecretString>, env: &EdenAiEnv) -> Option<SecretString> {
    api_key
        .filter(|key| !key.expose_secret().is_empty())
        .cloned()
        .or_else(|| environment_key(env))
}

// the explicit key, then litellm.api_key, then EDENAI_API_KEY; nothing found is the missing-key error
pub fn require_api_key(
    api_key: Option<&SecretString>,
    model: &str,
    env: &EdenAiEnv,
) -> Result<SecretString, MissingApiKey> {
    api_key
        .filter(|key| !key.expose_secret().is_empty())
        .cloned()
        .or_else(|| {
            env.global_api_key
                .as_ref()
                .filter(|key| !key.expose_secret().is_empty())
                .cloned()
        })
        .or_else(|| environment_key(env))
        .ok_or_else(|| MissingApiKey {
            model: model.to_owned(),
            status_code: 401,
        })
}

fn environment_key(env: &EdenAiEnv) -> Option<SecretString> {
    env.api_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty())
        .cloned()
}

// LiteLLM's authorized_headers: the caller's headers plus a Bearer token that replaces an Authorization
pub fn authorized_headers(
    headers: &Headers,
    api_key: Option<&SecretString>,
    model: &str,
    env: &EdenAiEnv,
) -> Result<Headers, MissingApiKey> {
    let key = require_api_key(api_key, model, env)?;
    let mut merged = removed(headers, AUTHORIZATION);
    merged.insert(
        AUTHORIZATION.to_owned(),
        format!("Bearer {}", key.expose_secret()),
    );
    Ok(merged)
}

// LiteLLM's json_headers: authorized_headers with a JSON content type, sent because the handler may send raw content
pub fn json_headers(
    headers: &Headers,
    api_key: Option<&SecretString>,
    model: &str,
    env: &EdenAiEnv,
) -> Result<Headers, MissingApiKey> {
    let authorized = authorized_headers(headers, api_key, model, env)?;
    let mut merged = removed(&authorized, CONTENT_TYPE);
    merged.insert(CONTENT_TYPE.to_owned(), JSON_CONTENT_TYPE.to_owned());
    Ok(merged)
}

// a caller value spelled in another case is dropped, as httpx would send the last of the two under one name
fn removed(headers: &Headers, name: &str) -> Headers {
    headers
        .iter()
        .filter(|(existing, _value)| !existing.eq_ignore_ascii_case(name))
        .map(|(existing, value)| (existing.clone(), value.clone()))
        .collect()
}

// the resolved base with its trailing slashes trimmed, then one slash and the path
#[must_use]
// ast-grep-ignore: no-padded-one-line-functions - the task names this shared helper and B5838 reuses it
pub fn endpoint_url(api_base: Option<&str>, path: &str, env: &EdenAiEnv) -> String {
    let base = resolve_api_base(api_base, env);
    format!("{}/{path}", base.trim_end_matches('/'))
}

// LiteLLM's pick: only the listed keys, kept in the order the params carry them
#[must_use]
pub fn pick(params: &Object, keys: &[&str]) -> Object {
    params
        .iter()
        .filter(|(name, _value)| keys.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// LiteLLM's reported_cost on a JSON value: the cost alias wins over the header alias, and an invalid value is no cost
#[must_use]
pub fn reported_cost(payload: &Value) -> Option<Decimal> {
    let Value::Object(fields) = payload else {
        return None;
    };
    cost_value(
        fields
            .get(COST)
            .or_else(|| fields.get(EDENAI_COST_HEADER))?,
    )
}

// any bytes from the provider: a body that is not a JSON object is no cost
#[must_use]
// ast-grep-ignore: no-padded-one-line-functions - the task names this decoder as the fuzz target's entry point
pub fn reported_cost_from_bytes(bytes: &[u8]) -> Option<Decimal> {
    let payload: Value = serde_json::from_slice(bytes).ok()?;
    reported_cost(&payload)
}

// a header map: both aliases are tried, and a name matches in any case because HTTP reads names that way
#[must_use]
pub fn reported_cost_from_headers(headers: &Headers) -> Option<Decimal> {
    let value =
        header_value(headers, COST).or_else(|| header_value(headers, EDENAI_COST_HEADER))?;
    cost_value(&Value::String(value))
}

fn header_value<'headers>(headers: &'headers Headers, name: &str) -> Option<&'headers String> {
    headers
        .iter()
        .find(|(existing, _value)| existing.eq_ignore_ascii_case(name))
        .map(|(_existing, value)| value)
}

// the pydantic model behind reported_cost: bools are 1 and 0, null and the wrong types are no cost
#[must_use]
pub fn cost_value(value: &Value) -> Option<Decimal> {
    match value {
        Value::Null => None,
        Value::Bool(flag) => Some(Decimal::from(u8::from(*flag))),
        Value::Number(number) => number::cost_decimal(&number.to_string()),
        Value::String(text) => number::cost_decimal(text),
        Value::Array(_) | Value::Object(_) => None,
    }
}

// LiteLLM's set_response_cost_in_hidden_params: no cost leaves the hidden params alone and keeps any caller headers
pub fn set_response_cost(hidden: &mut HiddenParams, cost: Option<Decimal>) {
    let Some(cost) = cost else {
        return;
    };
    let Some(number) = Number::from_str(&cost.normalize().to_string()).ok() else {
        return;
    };
    let value = HiddenJson::Number(number);
    if !matches!(hidden.get(ADDITIONAL_HEADERS), Some(HiddenJson::Object(_))) {
        hidden.insert(
            ADDITIONAL_HEADERS.to_owned(),
            HiddenJson::Object(AdditionalHeaders::new()),
        );
    }
    if let Some(HiddenJson::Object(headers)) = hidden.get_mut(ADDITIONAL_HEADERS) {
        headers.insert(RESPONSE_COST_HEADER.to_owned(), value);
    }
}

// the same record for a reply whose hidden headers are already the map itself, as a Responses reply keeps them
pub fn insert_response_cost(headers: &mut AdditionalHeaders, cost: Option<Decimal>) {
    let Some(cost) = cost else {
        return;
    };
    let Some(number) = Number::from_str(&cost.normalize().to_string()).ok() else {
        return;
    };
    headers.insert(RESPONSE_COST_HEADER.to_owned(), HiddenJson::Number(number));
}

// the fields the model catalog reply can fail on; LiteLLM's field name is kept in each message
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CatalogError {
    #[error("the Eden AI model catalog is not valid JSON")]
    Json,
    #[error("the Eden AI model catalog has no data list")]
    Data,
    #[error("the Eden AI model catalog entry has no id string")]
    Id,
}

// the /models body: {"data":[{"id":..}]}, and anything else is a typed error naming the missing field
pub fn parse_model_catalog(bytes: &[u8]) -> Result<Vec<String>, CatalogError> {
    let payload: Value = serde_json::from_slice(bytes).map_err(|_json| CatalogError::Json)?;
    let Value::Object(fields) = payload else {
        return Err(CatalogError::Data);
    };
    let Some(Value::Array(entries)) = fields.get("data") else {
        return Err(CatalogError::Data);
    };
    let mut ids = Vec::with_capacity(entries.len());
    for entry in entries {
        let Value::Object(fields) = entry else {
            return Err(CatalogError::Id);
        };
        let Some(Value::String(id)) = fields.get("id") else {
            return Err(CatalogError::Id);
        };
        ids.push(id.clone());
    }
    Ok(ids)
}

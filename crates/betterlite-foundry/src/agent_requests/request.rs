use std::borrow::Cow;
use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use super::error::AgentRequestError;
use super::messages::{ConvertedMessage, convert_messages};
use crate::credentials::{AZURE_AI_API_BASE_VAR, AZURE_AI_API_KEY_VAR};

pub const DEFAULT_API_VERSION: &str = "2025-05-01";

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

#[derive(Debug, Clone, Copy)]
pub struct AgentCallRequest<'data> {
    pub model: &'data str,
    pub messages: &'data [Object],
    pub optional_params: &'data Object,
    pub headers: &'data BTreeMap<String, String>,
    pub api_key: Option<&'data SecretString>,
    pub api_base: Option<&'data str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentRequestPayload {
    pub agent_id: String,
    pub messages: Vec<ConvertedMessage>,
    pub api_version: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Json>,
}

// LiteLLM's "explicit or AZURE_AI_API_BASE/AZURE_AI_API_KEY" chain: an empty explicit api_key reads the env too
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
) -> Result<ProviderInfo, AgentRequestError> {
    let base = match api_base.filter(|base| !base.is_empty()) {
        Some(base) => Some(String::from(base)),
        None => secret_text(secrets, environment, AZURE_AI_API_BASE_VAR)?
            .map(|text| text.expose_secret().to_owned()),
    };
    let key = match api_key.filter(|key| !key.expose_secret().is_empty()) {
        Some(key) => Some(key.clone()),
        None => secret_text(secrets, environment, AZURE_AI_API_KEY_VAR)?,
    };
    Ok(ProviderInfo {
        api_base: base,
        api_key: key,
    })
}

fn secret_text(
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
    name: &'static str,
) -> Result<Option<SecretString>, AgentRequestError> {
    secrets
        .get_secret_str(environment, name, None)
        .map_err(|source| AgentRequestError::Secret { name, source })
}

pub fn complete_url(api_base: Option<&str>) -> Result<String, AgentRequestError> {
    match api_base {
        None => Err(AgentRequestError::ApiBaseRequired),
        Some(base) => Ok(base.trim_end_matches('/').to_owned()),
    }
}

#[must_use]
pub fn agent_id_from_model(model: &str) -> String {
    match model.split_once("agents/") {
        Some((_, suffix)) => suffix.to_owned(),
        None => model.to_owned(),
    }
}

pub fn agent_id(model: &str, optional_params: &Object) -> Result<String, AgentRequestError> {
    for field in ["agent_id", "assistant_id"] {
        match optional_params.get(field) {
            None | Some(Json::Null) => {}
            Some(value) if !value.is_truthy() => {}
            Some(Json::Text(text)) => return Ok(text.clone()),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(AgentRequestError::FieldType { field });
            }
        }
    }
    Ok(agent_id_from_model(model))
}

#[must_use]
pub fn api_version(optional_params: &Object) -> Json {
    optional_params
        .get("api_version")
        .cloned()
        .unwrap_or_else(|| Json::Text(DEFAULT_API_VERSION.to_owned()))
}

// preparation interpolates the version the way Python's f-string does, str() of None included
pub fn api_version_text(api_version: &Json) -> Result<Cow<'_, str>, AgentRequestError> {
    match api_version {
        Json::Text(text) => Ok(Cow::Borrowed(text)),
        Json::Null => Ok(Cow::Borrowed("None")),
        Json::Bool(flag) => Ok(Cow::Borrowed(if *flag { "True" } else { "False" })),
        Json::Number(number) => Ok(Cow::Owned(number.to_string())),
        Json::List(_) | Json::Object(_) => Err(AgentRequestError::FieldType {
            field: "api_version",
        }),
    }
}

pub fn transform_request(
    request: &AgentCallRequest<'_>,
) -> Result<AgentRequestPayload, AgentRequestError> {
    Ok(AgentRequestPayload {
        agent_id: agent_id(request.model, request.optional_params)?,
        messages: convert_messages(request.messages)?,
        api_version: api_version(request.optional_params),
        thread_id: request.optional_params.get("thread_id").cloned(),
        instructions: request.optional_params.get("instructions").cloned(),
    })
}

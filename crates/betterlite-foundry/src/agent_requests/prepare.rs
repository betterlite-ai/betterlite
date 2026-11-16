use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::AgentRequestError;
use super::request::{AgentCallRequest, agent_id, api_version, api_version_text, complete_url};

const AUTHORIZATION_HEADER: &str = "Authorization";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";

pub type Headers = BTreeMap<String, SecretString>;

#[derive(Debug, Clone)]
pub struct PreparedAgentCall {
    pub headers: Headers,
    pub api_version: String,
    pub agent_id: String,
    pub thread_id: Option<String>,
    pub api_base: String,
}

// LiteLLM's validate_environment rewrites Content-Type and only a truthy key replaces an existing Authorization
#[must_use]
pub fn validate_environment(headers: &Headers, api_key: Option<&SecretString>) -> Headers {
    let mut validated = headers.clone();
    validated.insert(
        String::from(CONTENT_TYPE_HEADER),
        SecretString::from(JSON_CONTENT_TYPE),
    );
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        validated.insert(
            String::from(AUTHORIZATION_HEADER),
            SecretString::from(format!("Bearer {}", key.expose_secret())),
        );
    }
    validated
}

pub fn prepare(request: &AgentCallRequest<'_>) -> Result<PreparedAgentCall, AgentRequestError> {
    let api_base = complete_url(request.api_base)?;
    let headers = validate_environment(request.headers, request.api_key);
    let resolved_agent = agent_id(request.model, request.optional_params)?;
    let resolved_version = api_version_text(&api_version(request.optional_params))?.into_owned();
    let thread_id = match request.optional_params.get("thread_id") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(AgentRequestError::FieldType { field: "thread_id" });
        }
    };
    Ok(PreparedAgentCall {
        headers,
        api_version: resolved_version,
        agent_id: resolved_agent,
        thread_id,
        api_base,
    })
}

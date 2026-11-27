use betterlite_http::interactions::base::Params;
use betterlite_models::controls::json_value::JsonValue;
use secrecy::SecretString;

use super::error::GeminiAgentError;
use crate::studio_agents::request::{AgentParams, BaseEnvironment};

pub(super) fn agent_params(params: &Params) -> Result<AgentParams, GeminiAgentError> {
    Ok(AgentParams {
        api_key: text(params, "api_key")?.map(SecretString::from),
        api_base: text(params, "api_base")?,
        base_agent: text(params, "base_agent")?,
        instructions: text(params, "instructions")?,
        base_environment: match params.get("base_environment") {
            None | Some(JsonValue::Null) => None,
            Some(value) => Some(environment(value)?),
        },
        page_size: page_size(params)?,
        page_token: text(params, "page_token")?,
    })
}

fn text(params: &Params, field: &'static str) -> Result<Option<String>, GeminiAgentError> {
    match params.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(value)) => Ok(Some(value.clone())),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(GeminiAgentError::Params { field }),
    }
}

// LiteLLM compares page_size for truthiness; betterlite reads the integer the API declares
fn page_size(params: &Params) -> Result<Option<i64>, GeminiAgentError> {
    let Some(value) = params.get("page_size") else {
        return Ok(None);
    };
    if let JsonValue::Null = value {
        return Ok(None);
    }
    let JsonValue::Number(number) = value else {
        return Err(GeminiAgentError::Params { field: "page_size" });
    };
    number
        .as_i64()
        .map(Some)
        .ok_or(GeminiAgentError::Params { field: "page_size" })
}

// the native value goes through serde, so an object keeps every provider field it carries
fn environment(value: &JsonValue) -> Result<BaseEnvironment, GeminiAgentError> {
    let Ok(text) = serde_json::to_string(value) else {
        return Err(GeminiAgentError::Params {
            field: "base_environment",
        });
    };
    serde_json::from_str(&text).map_err(|_error| GeminiAgentError::Params {
        field: "base_environment",
    })
}

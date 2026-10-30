use super::AgentProviderError;
use crate::agents::json::AgentJson;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub type JsonRpcId = AgentJson;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiteLLMSendMessageResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<JsonRpcId>,
    #[serde(default)]
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<BTreeMap<String, AgentJson>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<BTreeMap<String, AgentJson>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<BTreeMap<String, AgentJson>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
    #[serde(skip)]
    pub _hidden_params: BTreeMap<String, AgentJson>,
}

impl LiteLLMSendMessageResponse {
    pub fn from_a2a_response(
        response: &Value,
        request_id: Option<&Value>,
    ) -> Result<Self, AgentProviderError> {
        let converted = Self::from_dict(response, request_id)?;
        Ok(converted)
    }

    pub fn from_dict(
        response: &Value,
        request_id: Option<&Value>,
    ) -> Result<Self, AgentProviderError> {
        let normalized = normalize_a2a_jsonrpc_response(response, request_id)?;
        serde_json::from_value(normalized).map_err(|source| AgentProviderError::Decode { source })
    }
}

pub fn normalize_a2a_jsonrpc_response(
    response: &Value,
    request_id: Option<&Value>,
) -> Result<Value, AgentProviderError> {
    let Value::Object(mut normalized) = response.clone() else {
        return Err(AgentProviderError::InvalidResponse {
            message: String::from("A2A JSON-RPC response must be an object"),
        });
    };
    match normalized.get("id") {
        Some(Value::Bool(value)) => {
            normalized.insert(String::from("id"), Value::String(value.to_string()));
        }
        Some(Value::Null) | None => {
            if let Some(request_id) = request_id {
                let id = match request_id {
                    Value::String(_) | Value::Number(_) => request_id.clone(),
                    other => Value::String(other.to_string()),
                };
                normalized.insert(String::from("id"), id);
            }
        }
        Some(Value::String(_) | Value::Number(_)) => {}
        Some(_) => {
            return Err(AgentProviderError::InvalidResponse {
                message: String::from("A2A JSON-RPC id must be a string or integer"),
            });
        }
    }
    Ok(Value::Object(normalized))
}

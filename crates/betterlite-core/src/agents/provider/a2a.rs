use super::AgentProviderError;
use crate::agents::json::AgentJson;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    String(String),
    Integer(IntegerId),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(try_from = "serde_json::Number")]
pub struct IntegerId(serde_json::Number);

impl TryFrom<serde_json::Number> for IntegerId {
    type Error = AgentProviderError;

    fn try_from(number: serde_json::Number) -> Result<Self, Self::Error> {
        if number.to_string().contains(['.', 'e', 'E']) {
            return Err(AgentProviderError::InvalidResponse {
                message: String::from("A2A JSON-RPC id must be an integer"),
            });
        }
        Ok(Self(number))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiteLLMSendMessageResponse {
    pub id: Option<JsonRpcId>,
    #[serde(default)]
    pub jsonrpc: Version,
    pub result: Option<BTreeMap<String, AgentJson>>,
    pub error: Option<BTreeMap<String, AgentJson>>,
    pub usage: Option<BTreeMap<String, AgentJson>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
    #[serde(skip)]
    pub hidden_params: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Version(pub String);

impl Default for Version {
    fn default() -> Self {
        Self(String::from("2.0"))
    }
}

impl LiteLLMSendMessageResponse {
    pub fn from_a2a_response(
        response: &AgentJson,
        request_id: Option<&AgentJson>,
    ) -> Result<Self, AgentProviderError> {
        let mut payload = response.clone();
        if let AgentJson::Object(fields) = &mut payload {
            fields.retain(|_, value| !value.is_null());
        }
        Self::from_dict(&payload, request_id)
    }

    pub fn from_dict(
        response: &AgentJson,
        request_id: Option<&AgentJson>,
    ) -> Result<Self, AgentProviderError> {
        let normalized = normalize_a2a_jsonrpc_response(response, request_id)?;
        serde_json::from_value(normalized).map_err(|source| AgentProviderError::Decode { source })
    }
}

pub fn normalize_a2a_jsonrpc_response(
    response: &AgentJson,
    request_id: Option<&AgentJson>,
) -> Result<AgentJson, AgentProviderError> {
    let AgentJson::Object(mut normalized) = response.clone() else {
        return Err(AgentProviderError::InvalidResponse {
            message: String::from("A2A JSON-RPC response must be an object"),
        });
    };
    match normalized.get("id") {
        Some(AgentJson::Bool(value)) => {
            let id = if *value { "True" } else { "False" };
            normalized.insert(String::from("id"), AgentJson::String(String::from(id)));
        }
        Some(AgentJson::Null) | None => {
            if let Some(id) = request_id.filter(|id| !id.is_null()) {
                let encoded = match id {
                    AgentJson::String(_) => id.clone(),
                    AgentJson::Number(number) if !number.to_string().contains(['.', 'e', 'E']) => {
                        id.clone()
                    }
                    other @ (AgentJson::Null
                    | AgentJson::Bool(_)
                    | AgentJson::Number(_)
                    | AgentJson::Array(_)
                    | AgentJson::Object(_)) => {
                        AgentJson::String(super::request_id::python_string(other)?)
                    }
                };
                normalized.insert(String::from("id"), encoded);
            }
        }
        Some(_) => {}
    }
    Ok(AgentJson::Object(normalized))
}

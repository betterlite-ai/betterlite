use betterlite_prompts::arguments::json::Json;
use serde::{Deserialize, Deserializer};
use thiserror::Error;

use super::messages::ContentItem;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentEvent {
    // "id" in data decides whether thread.created records one, so Some(Null) clears it and absence keeps it
    #[serde(default, deserialize_with = "present_json")]
    pub id: Option<Json>,
    #[serde(default)]
    pub content: Option<Vec<ContentItem>>,
    #[serde(default)]
    pub delta: Option<AgentDelta>,
}

fn present_json<'de, D>(deserializer: D) -> Result<Option<Json>, D::Error>
where
    D: Deserializer<'de>,
{
    Json::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentDelta {
    #[serde(default)]
    pub content: Option<Vec<ContentItem>>,
}

#[derive(Debug, Error)]
pub enum AgentEventError {
    #[error("the stream event data is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the stream event data is not a JSON object")]
    NotObject,
}

// LiteLLM's json.loads plus data.get(...) reaches only an object, so anything else is not this record
pub fn decode_agent_event(bytes: &[u8]) -> Result<AgentEvent, AgentEventError> {
    match serde_json::from_slice(bytes) {
        Ok(event) => Ok(event),
        Err(source) => match serde_json::from_slice::<Json>(bytes) {
            Ok(Json::Object(_)) => Err(AgentEventError::Json { source }),
            Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                Err(AgentEventError::NotObject)
            }
            Err(_) => Err(AgentEventError::Json { source }),
        },
    }
}

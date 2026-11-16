use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use thiserror::Error;

use super::messages::ContentItem;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentEvent {
    #[serde(default)]
    pub content: Option<Vec<ContentItem>>,
    #[serde(default)]
    pub delta: Option<AgentDelta>,
    // every other member, so a present id (even null) is told apart from an absent one
    #[serde(flatten)]
    pub fields: Object,
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

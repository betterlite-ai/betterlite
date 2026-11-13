use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::value::{RawValue, to_raw_value};

mod tests;

pub type JsonMap = IndexMap<String, Box<RawValue>>;

#[must_use]
pub fn get_text(map: &JsonMap, key: &str) -> Option<String> {
    if let Some(value) = map.get(key) {
        return serde_json::from_str(value.get()).ok();
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FollowupMessage(pub Box<RawValue>);

impl FollowupMessage {
    pub fn from_value<Payload>(value: &Payload) -> Result<Self, serde_json::Error>
    where
        Payload: Serialize,
    {
        match to_raw_value(value) {
            Ok(raw) => Ok(Self(raw)),
            Err(error) => Err(error),
        }
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgenticLoopRequestPatch {
    pub model: Option<String>,
    pub messages: Option<Vec<FollowupMessage>>,
    pub tools: Option<Vec<FollowupMessage>>,
    pub max_tokens: Option<u64>,
    pub optional_params: JsonMap,
    pub kwargs: JsonMap,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgenticLoopPlan {
    pub run_agentic_loop: bool,
    pub request_patch: Option<AgenticLoopRequestPatch>,
    pub response_override: Option<Box<RawValue>>,
    pub terminate: bool,
    pub stop_reason: Option<String>,
    pub metadata: JsonMap,
}

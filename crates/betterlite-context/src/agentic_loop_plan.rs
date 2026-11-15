use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

mod tests;

pub type JsonMap = IndexMap<String, Box<RawValue>>;

#[must_use]
pub fn get_text(map: &JsonMap, key: &str) -> Option<String> {
    let raw = map.get(key)?;
    let text = serde_json::from_str(raw.get()).ok()?;
    Some(text)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FollowupMessage(pub Box<RawValue>);

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

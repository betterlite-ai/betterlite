use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::databricks_chat::content::{DatabricksContent, DatabricksFunction};
use crate::databricks_chat::slot::Slot;

// The chunk dict chunk_parser reads; a key it reads with [] is a Slot so its absence is the KeyError
#[derive(Debug, Deserialize)]
pub(super) struct DatabricksChunk {
    #[serde(default)]
    pub id: Slot<String>,
    #[serde(default)]
    pub created: Slot<i64>,
    #[serde(default)]
    pub model: Slot<String>,
    #[serde(default)]
    pub choices: Slot<Vec<ChunkChoice>>,
    #[serde(default)]
    pub usage: Option<Object>,
    // isinstance(service_tier, str) decides, so any value is read and only a string is kept
    #[serde(default)]
    pub service_tier: Option<Json>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChunkChoice {
    #[serde(default)]
    pub delta: Slot<DatabricksDelta>,
    #[serde(default)]
    pub finish_reason: Option<String>,
    // a Number, since an i64 next to a flattened map fails under serde_json's arbitrary_precision
    #[serde(default)]
    pub index: Slot<Number>,
    // the provider's token log probabilities, forwarded as sent
    #[serde(default)]
    pub logprobs: Option<Json>,
    // StreamingChoices keeps the choice keys it does not name
    #[serde(flatten)]
    pub extra: Object,
}

// DatabricksDelta plus the keys Delta(**delta) consumes; every other key stays on the delta
#[derive(Debug, Deserialize)]
pub(super) struct DatabricksDelta {
    #[serde(default)]
    pub content: Option<DatabricksContent>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub reasoning: Slot<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
    #[serde(default)]
    pub function_call: Option<DeltaFunctionCall>,
    #[serde(default)]
    pub provider_specific_fields: Slot<Object>,
    // audio, images, annotations and reasoning items are OpenAI records Delta forwards as sent
    #[serde(default)]
    pub audio: Option<Json>,
    #[serde(default)]
    pub images: Option<Vec<Json>>,
    #[serde(default)]
    pub annotations: Option<Vec<Json>>,
    #[serde(default)]
    pub reasoning_items: Option<Vec<Json>>,
    #[serde(flatten)]
    pub extra: Object,
}

// ChatCompletionDeltaToolCall: an absent id stays absent while an explicit null dumps as null
#[derive(Debug, Deserialize)]
pub(super) struct DeltaToolCall {
    #[serde(default)]
    pub id: Slot<String>,
    #[serde(rename = "type", default)]
    pub kind: Slot<String>,
    #[serde(default)]
    pub index: Option<Number>,
    #[serde(default)]
    pub function: Slot<DatabricksFunction>,
    #[serde(default)]
    pub custom: Option<CustomPayload>,
    #[serde(flatten)]
    pub extra: Object,
}

// FunctionCall(**function_call): arguments is required, an unset name stays absent and extra keys stay
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct DeltaFunctionCall {
    pub arguments: String,
    #[serde(default, skip_serializing_if = "Slot::is_absent")]
    pub name: Slot<String>,
    #[serde(flatten)]
    pub extra: Object,
}

// ChatCompletionDeltaCustomToolCallPayload: an unset name or input stays absent, a null one dumps as null
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CustomPayload {
    #[serde(default, skip_serializing_if = "Slot::is_absent")]
    pub name: Slot<String>,
    #[serde(default, skip_serializing_if = "Slot::is_absent")]
    pub input: Slot<String>,
    #[serde(flatten)]
    pub extra: Object,
}

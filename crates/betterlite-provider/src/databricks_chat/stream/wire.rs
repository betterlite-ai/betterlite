use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_with::rust::double_option;

use crate::databricks_chat::content::{DatabricksContent, DatabricksFunction};

// The chunk dict chunk_parser reads; a key it reads with [] stays an Option<Option> so its absence is the KeyError
#[derive(Debug, Deserialize)]
pub(super) struct DatabricksChunk {
    #[serde(default, with = "double_option")]
    pub id: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    pub created: Option<Option<i64>>,
    #[serde(default, with = "double_option")]
    pub model: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    pub choices: Option<Option<Vec<ChunkChoice>>>,
    #[serde(default)]
    pub usage: Option<Object>,
    // isinstance(service_tier, str) decides, so any value is read and only a string is kept
    #[serde(default)]
    pub service_tier: Option<Json>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChunkChoice {
    #[serde(default, with = "double_option")]
    pub delta: Option<Option<DatabricksDelta>>,
    #[serde(default)]
    pub finish_reason: Option<String>,
    // a Number, since an i64 next to a flattened map fails under serde_json's arbitrary_precision
    #[serde(default, with = "double_option")]
    pub index: Option<Option<Number>>,
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
    #[serde(default, with = "double_option")]
    pub reasoning: Option<Option<String>>,
    #[serde(default)]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
    #[serde(default)]
    pub function_call: Option<DeltaFunctionCall>,
    #[serde(default, with = "double_option")]
    pub provider_specific_fields: Option<Option<Object>>,
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
    #[serde(default, with = "double_option")]
    pub id: Option<Option<String>>,
    #[serde(rename = "type", default, with = "double_option")]
    pub kind: Option<Option<String>>,
    #[serde(default)]
    pub index: Option<Number>,
    #[serde(default)]
    pub function: Option<Option<DatabricksFunction>>,
    #[serde(default)]
    pub custom: Option<CustomPayload>,
    #[serde(flatten)]
    pub extra: Object,
}

// FunctionCall(**function_call): arguments is required, an unset name stays absent and extra keys stay
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct DeltaFunctionCall {
    pub arguments: String,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<Option<String>>,
    #[serde(flatten)]
    pub extra: Object,
}

// ChatCompletionDeltaCustomToolCallPayload: an unset name or input stays absent, a null one dumps as null
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CustomPayload {
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<Option<String>>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub input: Option<Option<String>>,
    #[serde(flatten)]
    pub extra: Object,
}

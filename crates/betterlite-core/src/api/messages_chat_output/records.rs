use betterlite_models::decisions::Json as ModelJson;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer};

use crate::api::messages_context_clear::result::AppliedEdit;
use crate::api::messages_options::refusal::{ChatTurn, chat_refusal_text};

// LiteLLM's optional attributes read an absent key and an explicit null the same way, so both take the default.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

// LiteLLM reads each counter with getattr, so a provider can leave any JSON value on one.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UsageRecord {
    #[serde(default)]
    pub prompt_tokens: Option<i64>,
    #[serde(default)]
    pub completion_tokens: Option<i64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<Json>,
    #[serde(default, rename = "_cache_read_input_tokens")]
    pub private_cache_read_input_tokens: Option<Json>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<Json>,
    #[serde(default, rename = "_cache_creation_input_tokens")]
    pub private_cache_creation_input_tokens: Option<Json>,
    #[serde(default)]
    pub prompt_tokens_details: Option<Object>,
    // get_web_search_requests reads a plain mapping without validating it, so the accessor takes it as one.
    #[serde(default)]
    pub server_tool_use: Option<IndexMap<String, ModelJson>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FunctionRecord {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub provider_specific_fields: Object,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolCallRecord {
    #[serde(default)]
    pub id: Option<String>,
    // A null function is reachable in a stream chunk, and the pinned selection falls through it.
    #[serde(default)]
    pub function: Option<FunctionRecord>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub provider_specific_fields: Object,
}

// A present thinking_blocks attribute of any length differs from an absent one in stream chunks.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MessageRecord {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub tool_calls: Vec<ToolCallRecord>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub thinking_blocks: Option<Vec<Object>>,
    #[serde(default)]
    pub refusal: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub provider_specific_fields: Object,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeltaRecord {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub tool_calls: Vec<ToolCallRecord>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub thinking_blocks: Option<Vec<Object>>,
    #[serde(default)]
    pub refusal: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub provider_specific_fields: Object,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChoiceRecord {
    #[serde(default)]
    pub message: MessageRecord,
    #[serde(default)]
    pub delta: DeltaRecord,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponseRecord {
    pub id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<ChoiceRecord>,
    #[serde(default)]
    pub usage: Option<UsageRecord>,
}

// A streaming chunk's usage may sit on the response or in its hidden params.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TerminalRecord {
    #[serde(default)]
    pub choices: Vec<ChoiceRecord>,
    #[serde(default)]
    pub usage: Option<UsageRecord>,
    #[serde(default)]
    pub hidden_usage: Option<UsageRecord>,
    #[serde(default)]
    pub index: i64,
    #[serde(default)]
    pub applied_edits: Vec<AppliedEdit>,
}

fn model_json(value: &Json) -> ModelJson {
    match value {
        Json::Null => ModelJson::Null,
        Json::Bool(flag) => ModelJson::Bool(*flag),
        Json::Number(number) => ModelJson::Number(number.clone()),
        Json::Text(text) => ModelJson::Text(text.clone()),
        Json::List(items) => ModelJson::List(items.iter().map(model_json).collect()),
        Json::Object(fields) => ModelJson::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), model_json(item)))
                .collect(),
        ),
    }
}

impl MessageRecord {
    #[must_use]
    pub fn refusal_text(&self) -> Option<String> {
        refusal_text(self.refusal.as_deref(), &self.provider_specific_fields)
    }
}

impl DeltaRecord {
    #[must_use]
    pub fn refusal_text(&self) -> Option<String> {
        refusal_text(self.refusal.as_deref(), &self.provider_specific_fields)
    }
}

// B963 reads refusals from the models Json vocabulary while these records carry prompts Json.
fn refusal_text(refusal: Option<&str>, provider_specific_fields: &Object) -> Option<String> {
    let refusal_value = refusal.map_or(ModelJson::Null, |text| ModelJson::Text(text.to_owned()));
    let fields_value = ModelJson::Object(
        provider_specific_fields
            .iter()
            .map(|(key, value)| (key.clone(), model_json(value)))
            .collect(),
    );
    let turn = ChatTurn {
        refusal: &refusal_value,
        provider_specific_fields: &fields_value,
    };
    chat_refusal_text(&turn).map(ToOwned::to_owned)
}

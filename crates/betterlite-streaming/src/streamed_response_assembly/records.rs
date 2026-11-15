use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as};

use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};

// One already-parsed stream chunk: a plain dict or an SDK record flattened to the fields the builder reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Chunk {
    // LiteLLM keeps _hidden_params only while it is a dict; null and other values act absent
    #[serde(rename = "_hidden_params")]
    pub hidden_params: Option<JsonValue>,
    pub id: Option<String>,
    pub object: Option<String>,
    pub created: Option<i64>,
    pub model: Option<String>,
    pub system_fingerprint: Option<String>,
    pub service_tier: Option<String>,
    pub usage: Option<JsonValue>,
    pub choices: Vec<Choice>,
}

impl Chunk {
    #[must_use]
    pub fn hidden_object(&self) -> Option<&JsonObject> {
        let JsonValue::Object(fields) = self.hidden_params.as_ref()? else {
            return None;
        };
        Some(fields)
    }
}

// A chunk choice is a text choice when it carries a text key, even a null one, like TextChoices.
#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Choice {
    pub index: Option<u32>,
    pub finish_reason: Option<String>,
    pub logprobs: Option<JsonValue>,
    // LiteLLM reads a delta that is not a mapping as an empty one, so a null, string or list delta joins nothing
    #[serde_as(as = "DefaultOnError")]
    pub delta: Option<Delta>,
    pub text: TextField,
}

impl Choice {
    #[must_use]
    pub fn text_value(&self) -> Option<&str> {
        let TextField::Present(value) = &self.text else {
            return None;
        };
        value.as_str()
    }
}

// A missing text key and a null text value stay distinguishable, since only the key makes a text choice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TextField {
    Present(JsonValue),
    #[default]
    Absent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Delta {
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<JsonValue>,
    pub function_call: Option<JsonValue>,
    pub reasoning_content: Option<String>,
    pub thinking_blocks: Option<JsonValue>,
    pub annotations: Option<Vec<JsonValue>>,
    pub images: Option<Vec<JsonValue>>,
    pub audio: Option<JsonValue>,
    pub provider_specific_fields: Option<JsonValue>,
}

impl Delta {
    // The eight fields that turn the text-only fast path off whenever one of them is not None.
    #[must_use]
    pub const fn carries_more_than_text(&self) -> bool {
        self.tool_calls.is_some()
            || self.function_call.is_some()
            || self.reasoning_content.is_some()
            || self.thinking_blocks.is_some()
            || self.annotations.is_some()
            || self.audio.is_some()
            || self.images.is_some()
            || self.provider_specific_fields.is_some()
    }
}

impl Choice {
    #[must_use]
    pub fn provider_fields(&self) -> Option<&JsonObject> {
        let delta = self.delta.as_ref()?;
        let JsonValue::Object(fields) = delta.provider_specific_fields.as_ref()? else {
            return None;
        };
        Some(fields)
    }
}

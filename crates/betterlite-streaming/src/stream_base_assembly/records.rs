use serde::{Deserialize, Serialize};

use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};

// The fields the base response reads from one assembled stream chunk, from a plain dict or an SDK record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Chunk {
    // LiteLLM keeps _hidden_params only while it is a dict; null and other values act absent
    #[serde(rename = "_hidden_params")]
    pub hidden_params: Option<JsonValue>,
    pub object: Option<String>,
    pub id: Option<String>,
    pub created: Option<i64>,
    pub model: Option<String>,
    pub system_fingerprint: Option<String>,
    pub service_tier: Option<String>,
    pub choices: Vec<Choice>,
}

impl Chunk {
    #[must_use]
    pub const fn hidden_object(&self) -> Option<&JsonObject> {
        match self.hidden_params.as_ref() {
            Some(JsonValue::Object(fields)) => Some(fields),
            Some(
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::String(_)
                | JsonValue::Array(_),
            )
            | None => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Choice {
    pub delta: Option<Delta>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Delta {
    pub role: Option<String>,
}

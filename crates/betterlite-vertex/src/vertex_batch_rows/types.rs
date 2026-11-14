use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

// LiteLLM tests these fields with `in`, so an explicit null counts as present
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum RowField {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Value(Json),
}

impl RowField {
    #[must_use]
    pub const fn object(&self) -> Option<&IndexMap<String, Json>> {
        let Self::Value(Json::Object(fields)) = self else {
            return None;
        };
        Some(fields)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VertexBatchRow {
    pub key: Option<String>,
    pub request: RowField,
    pub status: Option<String>,
    pub processed_time: RowField,
    pub response: RowField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchKey {
    pub custom_id: String,
    pub index: u32,
    pub total: u32,
}

#[derive(Debug, Serialize)]
pub struct OpenAiBatchOutput {
    pub id: String,
    pub custom_id: String,
    pub response: Option<OpenAiBatchOutputResponse>,
    pub error: Option<OpenAiBatchOutputError>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiBatchOutputResponse {
    pub status_code: u16,
    pub request_id: Json,
    pub body: IndexMap<String, Json>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiBatchOutputError {
    pub code: String,
    pub message: String,
}

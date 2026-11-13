use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

// LiteLLM tests these fields with `in`, so a null value still counts as present
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RowField {
    Null,
    Value(Json),
}

#[derive(Debug, Clone, Deserialize)]
pub struct VertexBatchRow {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub request: Option<RowField>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub processed_time: Option<RowField>,
    #[serde(default)]
    pub response: Option<RowField>,
}

impl VertexBatchRow {
    // LiteLLM reads labels off a request that is a mapping and ignores any other shape
    #[must_use]
    pub fn labels(&self) -> Option<&IndexMap<String, Json>> {
        let Some(RowField::Value(Json::Object(request))) = self.request.as_ref() else {
            return None;
        };
        let Some(Json::Object(labels)) = request.get("labels") else {
            return None;
        };
        Some(labels)
    }

    #[must_use]
    pub const fn response_object(&self) -> Option<&IndexMap<String, Json>> {
        let Some(RowField::Value(Json::Object(response))) = self.response.as_ref() else {
            return None;
        };
        Some(response)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchKey {
    pub custom_id: String,
    pub index: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiBatchOutput {
    pub id: String,
    pub custom_id: String,
    pub response: Option<OpenAiBatchOutputResponse>,
    pub error: Option<OpenAiBatchOutputError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiBatchOutputResponse {
    pub status_code: u16,
    pub request_id: Json,
    pub body: Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiBatchOutputError {
    pub code: String,
    pub message: String,
}

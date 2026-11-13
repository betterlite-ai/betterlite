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

impl RowField {
    #[must_use]
    pub const fn value(&self) -> Option<&Json> {
        match self {
            Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }
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
        match self.request.as_ref().and_then(RowField::value) {
            Some(Json::Object(request)) => match request.get("labels") {
                Some(Json::Object(labels)) => Some(labels),
                Some(
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
                )
                | None => None,
            },
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => None,
        }
    }

    #[must_use]
    pub fn response_object(&self) -> Option<&IndexMap<String, Json>> {
        match self.response.as_ref().and_then(RowField::value) {
            Some(Json::Object(response)) => Some(response),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => None,
        }
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
    pub request_id: String,
    pub body: Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiBatchOutputError {
    pub code: String,
    pub message: String,
}

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat};
use serde::{Deserialize, Serialize};

use super::HeadersError;

// The injected catalog record carries more fields than the projection reads; the rest stay out of this view
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ModelRecord {
    pub id: String,
    #[serde(default)]
    pub max_input_tokens: Option<i64>,
    #[serde(default)]
    pub max_output_tokens: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelEntry {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_model: Option<String>,
    pub display_name: String,
    pub created_at: String,
    pub max_input_tokens: Option<i64>,
    #[serde(rename = "max_tokens")]
    pub max_output_tokens: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelListResponse {
    pub data: Vec<ModelEntry>,
    pub has_more: bool,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
}

#[must_use]
pub fn anthropic_model_entry(
    model: &ModelRecord,
    created_at: &str,
    display_names: &BTreeMap<String, String>,
    listed_ids: &BTreeMap<String, String>,
) -> ModelEntry {
    let listed_id = listed_ids.get(&model.id);
    let id = listed_id
        .filter(|replacement| !replacement.is_empty())
        .cloned()
        .unwrap_or_else(|| model.id.clone());
    ModelEntry {
        kind: "model",
        id,
        source_model: listed_id.map(|_| model.id.clone()),
        display_name: display_names
            .get(&model.id)
            .cloned()
            .unwrap_or_else(|| model.id.clone()),
        created_at: created_at.to_owned(),
        max_input_tokens: model.max_input_tokens,
        max_output_tokens: model.max_output_tokens,
    }
}

#[must_use]
pub fn create_anthropic_model_list_response(
    models: &[ModelRecord],
    display_names: &BTreeMap<String, String>,
    listed_ids: &BTreeMap<String, String>,
    created_at: &str,
) -> ModelListResponse {
    let data: Vec<ModelEntry> = models
        .iter()
        .map(|model| anthropic_model_entry(model, created_at, display_names, listed_ids))
        .collect();
    ModelListResponse {
        first_id: data.first().map(|entry| entry.id.clone()),
        last_id: data.last().map(|entry| entry.id.clone()),
        has_more: false,
        data,
    }
}

pub fn model_created_at(seconds: i64) -> Result<String, HeadersError> {
    let timestamp =
        DateTime::from_timestamp(seconds, 0).ok_or(HeadersError::Timestamp { seconds })?;
    Ok(timestamp.to_rfc3339_opts(SecondsFormat::Secs, true))
}

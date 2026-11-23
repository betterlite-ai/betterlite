use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::api::response_synthetic_events::python_value::has_content;

use super::access::PollOwner;
use super::error::UnknownPatchField;

// the public response map LiteLLM stores under the polling key; unknown fields stay in place
pub type PolledResponse = IndexMap<String, Json>;

// the architect's private envelope: the owner rides beside the response and never inside it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollRecord {
    pub owner: Option<PollOwner>,
    pub response: PolledResponse,
}

// request_data.metadata as the source reads it: absent defaults to {}, null stays null
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum PollMetadata {
    Fields(IndexMap<String, Json>),
    Null,
}

// the nineteen fields update_state takes; each value is written as given, whatever its type
const PATCH_FIELDS: &str = "status output usage error incomplete_details reasoning tool_choice tools \
     model instructions temperature top_p max_output_tokens previous_response_id text truncation \
     parallel_tool_calls user store";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(try_from = "IndexMap<String, Json>")]
pub struct PollPatch(pub(crate) IndexMap<String, Json>);

impl TryFrom<IndexMap<String, Json>> for PollPatch {
    type Error = UnknownPatchField;

    fn try_from(fields: IndexMap<String, Json>) -> Result<Self, Self::Error> {
        for field in fields.keys() {
            if !PATCH_FIELDS.split_whitespace().any(|name| name == field) {
                return Err(UnknownPatchField {
                    field: field.clone(),
                });
            }
        }
        Ok(Self(fields))
    }
}

impl PollPatch {
    // the source checks status, usage, error and incomplete_details for truthiness and the rest for presence
    pub fn apply(&self, response: &mut PolledResponse) {
        for (key, value) in &self.0 {
            let replace = match key.as_str() {
                "error" => false,
                "status" | "usage" | "incomplete_details" => has_content(value),
                _ => !matches!(value, Json::Null),
            };
            if replace {
                response.insert(key.clone(), value.clone());
            }
        }
        if let Some(error) = self.0.get("error").filter(|value| has_content(value)) {
            response.insert("status".to_owned(), Json::Text("failed".to_owned()));
            response.insert("error".to_owned(), error.clone());
        }
    }
}

// the queued record create_initial_state writes: the field set of B1138's ResponsesAPIResponse
#[must_use]
pub fn queued_response(
    polling_id: &str,
    created_at: i64,
    metadata: Option<&PollMetadata>,
) -> PolledResponse {
    let mut response = IndexMap::with_capacity(23);
    response.insert("id".to_owned(), Json::Text(polling_id.to_owned()));
    let created = Json::Number(Number::from(created_at));
    response.insert("created_at".to_owned(), created);
    response.insert("metadata".to_owned(), metadata_value(metadata));
    response.insert("object".to_owned(), Json::Text("response".to_owned()));
    response.insert("output".to_owned(), Json::List(Vec::new()));
    response.insert("status".to_owned(), Json::Text("queued".to_owned()));
    for key in PATCH_FIELDS.split_whitespace() {
        if !matches!(key, "status" | "output") {
            response.insert(key.to_owned(), Json::Null);
        }
    }
    response
}

fn metadata_value(metadata: Option<&PollMetadata>) -> Json {
    match metadata {
        None => Json::Object(IndexMap::new()),
        Some(PollMetadata::Null) => Json::Null,
        Some(PollMetadata::Fields(fields)) => Json::Object(fields.clone()),
    }
}

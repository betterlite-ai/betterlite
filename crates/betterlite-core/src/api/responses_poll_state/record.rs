use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::api::response_synthetic_events::python_value::has_content;

use super::owner::PollOwner;

// the public response map LiteLLM stores under the polling key; unknown fields stay in place
pub type PolledResponse = IndexMap<String, Json>;

// the architect's private envelope: the owner rides beside the response and never inside it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PollRecord {
    pub owner: Option<PollOwner>,
    pub response: PolledResponse,
}

// request_data.metadata as the source reads it: absent defaults to {}, null stays null
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum PollMetadata {
    Fields(IndexMap<String, Json>),
    Null,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PollPatch {
    pub status: Option<Json>,
    pub output: Option<Json>,
    pub usage: Option<Json>,
    pub error: Option<Json>,
    pub incomplete_details: Option<Json>,
    pub reasoning: Option<Json>,
    pub tool_choice: Option<Json>,
    pub tools: Option<Json>,
    pub model: Option<Json>,
    pub instructions: Option<Json>,
    pub temperature: Option<Json>,
    pub top_p: Option<Json>,
    pub max_output_tokens: Option<Json>,
    pub previous_response_id: Option<Json>,
    pub text: Option<Json>,
    pub truncation: Option<Json>,
    pub parallel_tool_calls: Option<Json>,
    pub user: Option<Json>,
    pub store: Option<Json>,
}

impl PollPatch {
    // the source mixes truthiness checks with presence checks; the order below is its order
    pub fn apply(&self, response: &mut PolledResponse) {
        apply_truthy(response, "status", self.status.as_ref());
        apply_present(response, "output", self.output.as_ref());
        apply_truthy(response, "usage", self.usage.as_ref());
        if let Some(error) = self.error.as_ref().filter(|value| has_content(value)) {
            response.insert("status".to_owned(), Json::Text("failed".to_owned()));
            response.insert("error".to_owned(), error.clone());
        }
        apply_truthy(
            response,
            "incomplete_details",
            self.incomplete_details.as_ref(),
        );
        apply_present(response, "reasoning", self.reasoning.as_ref());
        apply_present(response, "tool_choice", self.tool_choice.as_ref());
        apply_present(response, "tools", self.tools.as_ref());
        apply_present(response, "model", self.model.as_ref());
        apply_present(response, "instructions", self.instructions.as_ref());
        apply_present(response, "temperature", self.temperature.as_ref());
        apply_present(response, "top_p", self.top_p.as_ref());
        apply_present(
            response,
            "max_output_tokens",
            self.max_output_tokens.as_ref(),
        );
        apply_present(
            response,
            "previous_response_id",
            self.previous_response_id.as_ref(),
        );
        apply_present(response, "text", self.text.as_ref());
        apply_present(response, "truncation", self.truncation.as_ref());
        apply_present(
            response,
            "parallel_tool_calls",
            self.parallel_tool_calls.as_ref(),
        );
        apply_present(response, "user", self.user.as_ref());
        apply_present(response, "store", self.store.as_ref());
    }
}

fn apply_truthy(response: &mut PolledResponse, key: &str, value: Option<&Json>) {
    if let Some(value) = value.filter(|value| has_content(value)) {
        response.insert(key.to_owned(), value.clone());
    }
}

fn apply_present(response: &mut PolledResponse, key: &str, value: Option<&Json>) {
    match value {
        None | Some(Json::Null) => {}
        Some(value) => {
            response.insert(key.to_owned(), value.clone());
        }
    }
}

// the queued record create_initial_state writes: the field set and order of B1138's ResponsesAPIResponse
#[must_use]
pub fn queued_response(
    polling_id: &str,
    created_at: i64,
    metadata: Option<&PollMetadata>,
) -> PolledResponse {
    let mut response = IndexMap::with_capacity(23);
    response.insert("id".to_owned(), Json::Text(polling_id.to_owned()));
    response.insert(
        "created_at".to_owned(),
        Json::Number(Number::from(created_at)),
    );
    response.insert("error".to_owned(), Json::Null);
    response.insert("incomplete_details".to_owned(), Json::Null);
    response.insert("instructions".to_owned(), Json::Null);
    response.insert("metadata".to_owned(), metadata_value(metadata));
    response.insert("model".to_owned(), Json::Null);
    response.insert("object".to_owned(), Json::Text("response".to_owned()));
    response.insert("output".to_owned(), Json::List(Vec::new()));
    response.insert("parallel_tool_calls".to_owned(), Json::Null);
    response.insert("temperature".to_owned(), Json::Null);
    response.insert("tool_choice".to_owned(), Json::Null);
    response.insert("tools".to_owned(), Json::Null);
    response.insert("top_p".to_owned(), Json::Null);
    response.insert("max_output_tokens".to_owned(), Json::Null);
    response.insert("previous_response_id".to_owned(), Json::Null);
    response.insert("reasoning".to_owned(), Json::Null);
    response.insert("status".to_owned(), Json::Text("queued".to_owned()));
    response.insert("text".to_owned(), Json::Null);
    response.insert("truncation".to_owned(), Json::Null);
    response.insert("usage".to_owned(), Json::Null);
    response.insert("user".to_owned(), Json::Null);
    response.insert("store".to_owned(), Json::Null);
    response
}

fn metadata_value(metadata: Option<&PollMetadata>) -> Json {
    match metadata {
        None => Json::Object(IndexMap::new()),
        Some(PollMetadata::Null) => Json::Null,
        Some(PollMetadata::Fields(fields)) => Json::Object(fields.clone()),
    }
}

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::builders::{
    SearchInput, SearchResult, build_web_search_call,
};
use indexmap::IndexMap;

use super::state::StreamState;
use super::wire::{self, JsonFields};

#[must_use]
pub fn web_search_call_snapshot(state: &StreamState) -> IndexMap<String, Json> {
    let mut calls = IndexMap::with_capacity(state.web_search_calls.len());
    for (id, call) in &state.web_search_calls {
        calls.insert(id.clone(), call.clone());
    }
    calls
}

pub fn complete_web_search_call(state: &mut StreamState, result: &Json) {
    let Json::Object(fields) = result else {
        return;
    };
    let Some(Json::Text(tool_use_id)) = fields.get("tool_use_id") else {
        return;
    };
    if !state.web_search_calls.contains_key(tool_use_id) {
        return;
    }
    let tool_input = state
        .server_tool_inputs
        .get(tool_use_id)
        .cloned()
        .unwrap_or_else(|| Json::Object(JsonFields::new()));
    let Some(call) = web_search_call_json(tool_use_id, &tool_input, result) else {
        return;
    };
    state.web_search_calls.insert(tool_use_id.clone(), call);
}

fn web_search_call_json(tool_id: &str, tool_input: &Json, result: &Json) -> Option<Json> {
    let input = wire::decode_field::<SearchInput>(tool_input)?;
    let output = wire::decode_field::<SearchResult>(result)?;
    wire::to_json(&build_web_search_call(tool_id, &input, &output, None))
}

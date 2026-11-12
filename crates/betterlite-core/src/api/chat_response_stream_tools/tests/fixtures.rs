use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use indexmap::IndexMap;

use super::super::state::{ChatStreamState, StreamSetup};
use crate::api::api_boundaries::ApiLimits;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

pub(super) fn request(tools: Json) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(String::from("tools"), tools);
    Json::Object(fields)
}

pub(super) fn state_with(tools: Json) -> ChatStreamState<String> {
    state_with_metadata(tools, None)
}

pub(super) fn state_with_metadata(
    tools: Json,
    litellm_metadata: Option<IndexMap<String, Json>>,
) -> ChatStreamState<String> {
    let request = request(tools);
    ChatStreamState::new(StreamSetup {
        model: "gpt-4o-mini",
        logging_context: String::from("wrapper-context"),
        request_input: &Json::Text(String::from("hi")),
        responses_api_request: &request,
        custom_llm_provider: Some("openai"),
        litellm_metadata,
        limits: ApiLimits::default(),
    })
    .expect("the state builds")
}

pub(super) fn events(state: &ChatStreamState<String>) -> Vec<Json> {
    state
        .progress
        .pending_tool_events
        .iter()
        .map(|queued| json(&serde_json::to_string(&queued.event).expect("the event serializes")))
        .collect()
}

pub(super) fn deltas(arguments: &str) -> Json {
    let call =
        format!(r#"{{"id":"c","index":0,"function":{{"name":"f","arguments":"{arguments}"}}}}"#);
    Json::List(vec![json(&call)])
}

pub(super) fn output_item(text: &str) -> ResponseOutputItem {
    serde_json::from_str(text).expect("the fixture is a response output item")
}

pub(super) fn item_id(item: &ResponseOutputItem) -> Json {
    let text = serde_json::to_string(item).expect("the item serializes");
    match json(&text) {
        Json::Object(fields) => fields.get("id").cloned().unwrap_or(Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Json::Null,
    }
}

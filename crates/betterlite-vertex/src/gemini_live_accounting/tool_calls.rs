use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{As, DefaultOnNull};
use uuid::Uuid;

use crate::gemini_live_client::state::ToolCallNames;

const GENERATED_CALL_ID_DIGITS: usize = 16;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallMessage {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub function_calls: Vec<FunctionCall>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FunctionCall {
    pub id: Option<String>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub name: String,
    #[serde(default = "default_args")]
    pub args: Json,
}

fn default_args() -> Json {
    Json::Object(IndexMap::new())
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ToolCallIds<'ids> {
    pub response_id: Option<&'ids str>,
    pub output_item_id: Option<&'ids str>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FunctionCallArgumentsDone {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub event_id: String,
    pub response_id: String,
    pub item_id: String,
    pub output_index: usize,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

pub fn transform_tool_call_events(
    names: &mut ToolCallNames,
    message: &ToolCallMessage,
    ids: ToolCallIds<'_>,
    uuids: &mut dyn FnMut() -> Uuid,
) -> Vec<FunctionCallArgumentsDone> {
    let response_id = ids
        .response_id
        .map_or_else(|| format!("resp_{}", uuids()), str::to_owned);
    let output_item_id = ids
        .output_item_id
        .map_or_else(|| format!("item_{}", uuids()), str::to_owned);
    let mut events = Vec::with_capacity(message.function_calls.len());
    for (index, call) in message.function_calls.iter().enumerate() {
        let call_id = call.id.as_deref().filter(|id| !id.is_empty()).map_or_else(
            || {
                let digits: String = uuids()
                    .simple()
                    .to_string()
                    .chars()
                    .take(GENERATED_CALL_ID_DIGITS)
                    .collect();
                format!("call_{digits}")
            },
            str::to_owned,
        );
        names.insert(&call_id, &call.name);
        events.push(FunctionCallArgumentsDone {
            event_type: "response.function_call_arguments.done",
            event_id: format!("event_{}", uuids()),
            response_id: response_id.clone(),
            item_id: format!("{output_item_id}_tool_{index}"),
            output_index: index,
            call_id,
            name: call.name.clone(),
            arguments: dumps(&call.args),
        });
    }
    events
}

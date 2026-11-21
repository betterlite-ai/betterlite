use std::convert::Infallible;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use indexmap::IndexMap;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, ItemsBoundary};
use crate::api::chat_response_stream_tools::fields::string_value;
use crate::api::responses_file_search_tools::error::FileSearchToolsError;

use super::error::FileSearchLoopError;

// _build_follow_up_input (L489-L512): caller items, first output items with nulls dropped, results
pub fn build_follow_up_input(
    input: &Json,
    first_output: &[ResponseOutputItem],
    tool_results: &[Json],
    limits: ApiLimits,
) -> Result<Json, FileSearchLoopError<Infallible>> {
    let mut items = original_items(input);
    let known_count = items
        .len()
        .saturating_add(first_output.len())
        .saturating_add(tool_results.len());
    limits.check(ApiBoundariesInput::Items(ItemsBoundary { known_count }))?;
    for item in first_output {
        items.push(follow_up_item(item)?);
    }
    items.extend(tool_results.iter().cloned());
    Ok(Json::List(items))
}

// list(input) if isinstance(input, (list, tuple)) else [{"role": "user", "content": str(input)}]
fn original_items(input: &Json) -> Vec<Json> {
    match input {
        Json::List(items) => items.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            vec![user_message(&string_value(input))]
        }
    }
}

fn user_message(content: &str) -> Json {
    Json::Object(IndexMap::from([
        (String::from("role"), Json::Text(String::from("user"))),
        (String::from("content"), Json::Text(String::from(content))),
    ]))
}

// a plain dict item is kept as it is; a model item dumps with exclude_none=True off the wire form
fn follow_up_item(item: &ResponseOutputItem) -> Result<Json, FileSearchToolsError> {
    let text =
        serde_json::to_string(item).map_err(|source| FileSearchToolsError::Wire { source })?;
    let mut value: Json =
        serde_json::from_str(&text).map_err(|source| FileSearchToolsError::Wire { source })?;
    match item {
        ResponseOutputItem::Unknown(_) => {}
        ResponseOutputItem::Sdk(_)
        | ResponseOutputItem::Message(_)
        | ResponseOutputItem::FunctionCall(_)
        | ResponseOutputItem::ImageGeneration(_)
        | ResponseOutputItem::CodeInterpreter(_)
        | ResponseOutputItem::CustomTool(_) => strip_null_values(&mut value),
    }
    Ok(value)
}

fn strip_null_values(value: &mut Json) {
    match value {
        Json::List(items) => {
            for item in items {
                strip_null_values(item);
            }
        }
        Json::Object(fields) => {
            fields.retain(|_key, field| !matches!(field, Json::Null));
            for field in fields.values_mut() {
                strip_null_values(field);
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}

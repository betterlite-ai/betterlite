use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_mcp_results::McpToolResult;

mod tests;

#[derive(Debug, Error)]
pub enum FollowUpInputError {
    #[error("a Responses output item could not be copied into the follow-up input: {source}")]
    Item {
        #[from]
        source: serde_json::Error,
    },
}

// LiteLLM's `store is False`: an absent or null store and store=true all leave persistence on
#[must_use]
pub const fn is_persistence_disabled(call: &ResponsesCreateCall) -> bool {
    match call.store {
        Some(store) => !store,
        None => false,
    }
}

pub fn create_follow_up_input(
    response: &ResponsesApiResponse,
    tool_results: &[McpToolResult],
    original_input: Option<&ResponsesInput>,
    preserve_reasoning: bool,
) -> Result<Vec<ResponsesItem>, FollowUpInputError> {
    let mut items = original_items(original_input);
    let mut content = Vec::new();
    let mut turn_items = Vec::new();
    for output_item in &response.output {
        append_output_item(
            output_item,
            preserve_reasoning,
            &mut content,
            &mut turn_items,
        )?;
    }
    if !content.is_empty() {
        items.push(ResponsesItem(assistant_message(content)));
    }
    items.extend(turn_items);
    items.extend(tool_results.iter().map(tool_output));
    Ok(items)
}

fn original_items(input: Option<&ResponsesInput>) -> Vec<ResponsesItem> {
    let Some(original) = input else {
        return Vec::new();
    };
    match original {
        ResponsesInput::Text(text) if text.is_empty() => Vec::new(),
        ResponsesInput::Text(text) => vec![user_message(text)],
        ResponsesInput::Items(items) => items.clone(),
    }
}

fn user_message(text: &str) -> ResponsesItem {
    ResponsesItem(Json::Object(Object::from([
        (String::from("type"), Json::Text(String::from("message"))),
        (String::from("role"), Json::Text(String::from("user"))),
        (String::from("content"), Json::Text(text.to_owned())),
    ])))
}

fn assistant_message(content: Vec<Json>) -> Json {
    Json::Object(Object::from([
        (String::from("type"), Json::Text(String::from("message"))),
        (String::from("role"), Json::Text(String::from("assistant"))),
        (String::from("content"), Json::List(content)),
    ]))
}

fn append_output_item(
    item: &ResponseOutputItem,
    preserve_reasoning: bool,
    content: &mut Vec<Json>,
    turn_items: &mut Vec<ResponsesItem>,
) -> Result<(), FollowUpInputError> {
    let value = follow_up_value(item)?;
    let kind = item_type(&value);
    match kind.as_deref() {
        Some("message") => append_content(&value, content),
        Some("function_call") => {
            if let Some(call) = function_call_item(&value) {
                turn_items.push(ResponsesItem(call));
            }
        }
        Some("reasoning") if preserve_reasoning => turn_items.push(ResponsesItem(value)),
        Some(_) | None => {}
    }
    Ok(())
}

fn item_type(value: &Json) -> Option<String> {
    let Json::Object(fields) = value else {
        return None;
    };
    let Some(Json::Text(kind)) = fields.get("type") else {
        return None;
    };
    Some(kind.clone())
}

fn append_content(value: &Json, content: &mut Vec<Json>) {
    let Json::Object(fields) = value else {
        return;
    };
    match fields.get("content") {
        Some(Json::List(parts)) => content.extend(parts.iter().cloned()),
        Some(part) => content.push(part.clone()),
        None => {}
    }
}

fn function_call_item(value: &Json) -> Option<Json> {
    let Json::Object(fields) = value else {
        return None;
    };
    let call_id = text_field(fields, "call_id").or_else(|| text_field(fields, "id"))?;
    let name = text_field(fields, "name")?;
    let arguments = fields.get("arguments").cloned().unwrap_or(Json::Null);
    Some(Json::Object(Object::from([
        (
            String::from("type"),
            Json::Text(String::from("function_call")),
        ),
        (String::from("call_id"), Json::Text(call_id)),
        (String::from("name"), Json::Text(name)),
        (String::from("arguments"), arguments),
    ])))
}

fn text_field(fields: &Object, name: &str) -> Option<String> {
    let Some(Json::Text(text)) = fields.get(name) else {
        return None;
    };
    if text.is_empty() {
        return None;
    }
    Some(text.clone())
}

fn tool_output(result: &McpToolResult) -> ResponsesItem {
    let call_id = result
        .tool_call_id
        .as_ref()
        .map_or(Json::Null, |id| Json::Text(id.clone()));
    ResponsesItem(Json::Object(Object::from([
        (
            String::from("type"),
            Json::Text(String::from("function_call_output")),
        ),
        (String::from("call_id"), call_id),
        (String::from("output"), Json::Text(result.result.clone())),
    ])))
}

// a typed item is copied like model_dump(exclude_none=True); B1138's Unknown mapping is copied as it arrived
fn follow_up_value(item: &ResponseOutputItem) -> Result<Json, FollowUpInputError> {
    let text = serde_json::to_string(item)?;
    let value: Json = serde_json::from_str(&text)?;
    match item {
        ResponseOutputItem::Unknown(_) => Ok(value),
        ResponseOutputItem::Sdk(_)
        | ResponseOutputItem::Message(_)
        | ResponseOutputItem::FunctionCall(_)
        | ResponseOutputItem::ImageGeneration(_)
        | ResponseOutputItem::CodeInterpreter(_)
        | ResponseOutputItem::CustomTool(_) => Ok(without_nulls(value)),
    }
}

fn without_nulls(value: Json) -> Json {
    match value {
        Json::Object(fields) => Json::Object(
            fields
                .into_iter()
                .filter(|(_name, field)| !matches!(field, Json::Null))
                .map(|(name, field)| (name, without_nulls(field)))
                .collect(),
        ),
        Json::List(items) => Json::List(items.into_iter().map(without_nulls).collect()),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)) => other,
    }
}

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;
use betterlite_openai::api::responses_shapes::GenericResponseOutputItem;

fn is_empty_value(value: &Json) -> bool {
    match value {
        Json::Null | Json::Bool(false) => true,
        Json::Bool(true) => false,
        Json::Number(number) => number.as_f64() == Some(0.0),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(entries) => entries.is_empty(),
    }
}

fn text_of(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn message_call_id(message: &GenericResponseOutputItem) -> Option<&str> {
    if message.r#type != "function_call" {
        return None;
    }
    message
        .extra
        .get("call_id")
        .filter(|value| !is_empty_value(value))
        .map_or(Some(message.id.as_str()), text_of)
}

fn entries_call_id(entries: &BTreeMap<String, Json>) -> Option<&str> {
    if entries.get("type").and_then(text_of) != Some("function_call") {
        return None;
    }
    entries
        .get("call_id")
        .filter(|value| !is_empty_value(value))
        .or_else(|| entries.get("id"))
        .and_then(text_of)
}

#[must_use]
pub fn function_call_id(item: &ResponseOutputItem) -> Option<&str> {
    match item {
        ResponseOutputItem::Sdk(output) => {
            let SdkOutput::FunctionCall(call) = output.as_ref() else {
                return None;
            };
            Some(call.call_id.as_str())
                .filter(|text| !text.is_empty())
                .or(call.id.as_deref())
        }
        ResponseOutputItem::FunctionCall(call) => {
            if call.r#type.as_deref() != Some("function_call") {
                return None;
            }
            call.call_id
                .as_deref()
                .filter(|text| !text.is_empty())
                .or(call.id.as_deref())
        }
        ResponseOutputItem::Message(message) => message_call_id(message),
        ResponseOutputItem::Unknown(entries) => entries_call_id(entries),
        ResponseOutputItem::ImageGeneration(_)
        | ResponseOutputItem::CodeInterpreter(_)
        | ResponseOutputItem::CustomTool(_) => None,
    }
}

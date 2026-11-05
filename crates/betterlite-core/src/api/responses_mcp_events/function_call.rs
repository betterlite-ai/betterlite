use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;

fn text_entry<'entries>(
    entries: &'entries BTreeMap<String, Json>,
    key: &str,
) -> Option<&'entries str> {
    match entries.get(key) {
        Some(Json::Text(text)) => Some(text),
        _ => None,
    }
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
        ResponseOutputItem::Message(message) => {
            if message.r#type != "function_call" {
                return None;
            }
            text_entry(&message.extra, "call_id")
                .filter(|text| !text.is_empty())
                .or(Some(message.id.as_str()))
        }
        ResponseOutputItem::Unknown(entries) => {
            if text_entry(entries, "type") != Some("function_call") {
                return None;
            }
            text_entry(entries, "call_id")
                .filter(|text| !text.is_empty())
                .or_else(|| text_entry(entries, "id"))
        }
        ResponseOutputItem::ImageGeneration(_)
        | ResponseOutputItem::CodeInterpreter(_)
        | ResponseOutputItem::CustomTool(_) => None,
    }
}

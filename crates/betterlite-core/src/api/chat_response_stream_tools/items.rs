use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;
use betterlite_openai::api::responses_shapes::{
    CustomToolCallOutputItem, GenericResponseOutputItem, OutputCodeInterpreterCall,
    OutputFunctionToolCall, OutputImageGenerationCall,
};

use super::fields::object;
use crate::api::responses_tool_namespaces::values::has_value;

const IMAGE_GENERATION_CALL: &str = "image_generation_call";
const CODE_INTERPRETER_CALL: &str = "code_interpreter_call";
const CUSTOM_TOOL_CALL: &str = "custom_tool_call";

#[must_use]
pub fn index_of_output_item_type(items: &[ResponseOutputItem], item_type: &str) -> Option<usize> {
    items
        .iter()
        .position(|item| item_type_of(item).as_deref() == Some(item_type))
}

#[must_use]
pub fn output_items_with_id(
    items: &[ResponseOutputItem],
    item_type: &str,
    item_id: Option<&str>,
) -> Vec<ResponseOutputItem> {
    let Some(replacement) = item_id else {
        return items.to_vec();
    };
    let Some(target) = index_of_output_item_type(items, item_type) else {
        return items.to_vec();
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            if index == target {
                item_with_id(item, replacement)
            } else {
                item.clone()
            }
        })
        .collect()
}

// LiteLLM reads getattr(item, "type"): a dict kept as provider JSON has no attribute, so Unknown never matches
fn item_type_of(item: &ResponseOutputItem) -> Option<String> {
    match item {
        ResponseOutputItem::Sdk(output) => sdk_type(output),
        ResponseOutputItem::Message(message) => Some(message.r#type.clone()),
        ResponseOutputItem::FunctionCall(call) => call.r#type.clone(),
        ResponseOutputItem::ImageGeneration(_) => Some(IMAGE_GENERATION_CALL.to_owned()),
        ResponseOutputItem::CodeInterpreter(_) => Some(CODE_INTERPRETER_CALL.to_owned()),
        ResponseOutputItem::CustomTool(_) => Some(CUSTOM_TOOL_CALL.to_owned()),
        ResponseOutputItem::Unknown(_) => None,
    }
}

fn sdk_type(output: &SdkOutput) -> Option<String> {
    let value = serde_json::to_value(output).ok()?;
    let item_type = value.get("type")?;
    item_type.as_str().map(str::to_owned)
}

fn item_with_id(item: &ResponseOutputItem, item_id: &str) -> ResponseOutputItem {
    match item {
        ResponseOutputItem::Sdk(output) => sdk_item_with_id(output, item_id).map_or_else(
            || item.clone(),
            |updated| ResponseOutputItem::Sdk(Box::new(updated)),
        ),
        ResponseOutputItem::Message(message) => {
            ResponseOutputItem::Message(GenericResponseOutputItem {
                id: item_id.to_owned(),
                ..message.clone()
            })
        }
        ResponseOutputItem::FunctionCall(call) => {
            ResponseOutputItem::FunctionCall(OutputFunctionToolCall {
                id: Some(item_id.to_owned()),
                ..call.clone()
            })
        }
        ResponseOutputItem::ImageGeneration(call) => {
            ResponseOutputItem::ImageGeneration(OutputImageGenerationCall {
                id: item_id.to_owned(),
                ..call.clone()
            })
        }
        ResponseOutputItem::CodeInterpreter(call) => {
            ResponseOutputItem::CodeInterpreter(OutputCodeInterpreterCall {
                id: item_id.to_owned(),
                ..call.clone()
            })
        }
        ResponseOutputItem::CustomTool(call) => {
            ResponseOutputItem::CustomTool(CustomToolCallOutputItem {
                id: Some(item_id.to_owned()),
                ..call.clone()
            })
        }
        ResponseOutputItem::Unknown(_) => item.clone(),
    }
}

// every one of the SDK union's variants calls its id "id" on the wire, so one rewrite covers them all
fn sdk_item_with_id(output: &SdkOutput, item_id: &str) -> Option<SdkOutput> {
    let mut value = serde_json::to_value(output).ok()?;
    value
        .as_object_mut()?
        .insert(String::from("id"), serde_json::json!(item_id));
    serde_json::from_value(value).ok()
}

#[must_use]
pub fn has_signed_thinking_block(delta: &Json) -> bool {
    let Some(blocks) = object(delta).and_then(|fields| fields.get("thinking_blocks")) else {
        return false;
    };
    let Json::List(listed) = blocks else {
        return false;
    };
    listed.iter().any(|block| {
        object(block).is_some_and(|fields| {
            fields.get("signature").is_some_and(has_value)
                || fields.get("data").is_some_and(has_value)
        })
    })
}

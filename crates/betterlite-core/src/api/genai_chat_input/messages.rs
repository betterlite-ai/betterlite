use std::slice;

use betterlite_openai::chat::{
    ChatMessage, ContentPart, FunctionCall, MessageContent, Role, ToolCall,
};
use indexmap::IndexMap;
use serde_json::Value;

use super::GenAiInputError;
use super::input::{Call, Content, Contents, InlineData, Part, PartFields, SystemInstruction};

pub(super) fn messages(
    contents: &Contents,
    system: Option<&SystemInstruction>,
) -> Result<Vec<ChatMessage>, GenAiInputError> {
    let mut messages = Vec::new();
    let system_text = system
        .and_then(|instruction| instruction.parts.as_deref())
        .and_then(|parts| parts.first())
        .and_then(|part| part.text.as_deref());
    if let Some(text) = system_text {
        messages.push(ChatMessage {
            role: Role::System,
            content: Some(MessageContent::Text(text.to_owned())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            passthrough: IndexMap::new(),
        });
    }
    let items = match contents {
        Contents::One(content) => slice::from_ref(content),
        Contents::Many(many) => many,
    };
    for content in items {
        match content.role.as_deref().unwrap_or("user") {
            "user" => user_messages(content, &mut messages)?,
            "model" => messages.push(model_message(content)?),
            // LiteLLM transforms only the user and model roles and drops every other role without a word
            _other => {}
        }
    }
    Ok(messages)
}

fn user_messages(
    content: &Content,
    messages: &mut Vec<ChatMessage>,
) -> Result<(), GenAiInputError> {
    let mut parts = Vec::new();
    let mut tool_messages = Vec::new();
    for part in content.parts.as_deref().unwrap_or_default() {
        match part {
            Part::Text(text) => parts.push(text_part(text)),
            Part::Fields(fields) => user_fields(fields, &mut parts, &mut tool_messages)?,
            Part::Other(_) => {}
        }
    }
    if !parts.is_empty() {
        let message_content = match parts.as_slice() {
            [part] if part.kind == "text" => MessageContent::Text(
                part.fields
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ),
            _ => MessageContent::Parts(parts),
        };
        messages.push(ChatMessage {
            role: Role::User,
            content: Some(message_content),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            passthrough: IndexMap::new(),
        });
    }
    messages.extend(tool_messages);
    Ok(())
}

fn user_fields(
    fields: &PartFields,
    parts: &mut Vec<ContentPart>,
    tool_messages: &mut Vec<ChatMessage>,
) -> Result<(), GenAiInputError> {
    if let Some(text) = fields.text.as_deref() {
        parts.push(text_part(text));
        return Ok(());
    }
    if let Some(inline) = &fields.inline_data {
        parts.push(image_part(inline));
        return Ok(());
    }
    if let Some(response) = &fields.function_response {
        tool_messages.push(tool_message(response)?);
    }
    Ok(())
}

fn text_part(text: &str) -> ContentPart {
    ContentPart {
        kind: String::from("text"),
        fields: IndexMap::from([(String::from("text"), Value::String(text.to_owned()))]),
    }
}

fn image_part(inline: &InlineData) -> ContentPart {
    let mime = inline.mime_type.as_deref().unwrap_or("image/jpeg");
    let data = inline.data.as_deref().unwrap_or_default();
    let mut image_url = serde_json::Map::new();
    drop(image_url.insert(
        String::from("url"),
        Value::String(format!("data:{mime};base64,{data}")),
    ));
    ContentPart {
        kind: String::from("image_url"),
        fields: IndexMap::from([(String::from("image_url"), Value::Object(image_url))]),
    }
}

fn tool_message(response: &Call) -> Result<ChatMessage, GenAiInputError> {
    let name = response.name.as_deref().unwrap_or("unknown");
    Ok(ChatMessage {
        role: Role::Tool,
        content: Some(MessageContent::Text(encode_json(
            response.response.as_ref(),
        )?)),
        name: None,
        tool_calls: None,
        tool_call_id: Some(format!("call_{name}")),
        passthrough: IndexMap::new(),
    })
}

fn encode_json(value: Option<&Value>) -> Result<String, GenAiInputError> {
    let Some(given) = value else {
        return Ok(String::from("{}"));
    };
    Ok(serde_json::to_string(given)?)
}

fn model_message(content: &Content) -> Result<ChatMessage, GenAiInputError> {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for part in content.parts.as_deref().unwrap_or_default() {
        match part {
            Part::Text(part_text) => text.push_str(part_text),
            Part::Fields(fields) => model_fields(fields, &mut text, &mut tool_calls)?,
            Part::Other(_) => {}
        }
    }
    Ok(ChatMessage {
        role: Role::Assistant,
        content: (!text.is_empty()).then(|| MessageContent::Text(text)),
        name: None,
        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
        tool_call_id: None,
        passthrough: IndexMap::new(),
    })
}

fn model_fields(
    fields: &PartFields,
    text: &mut String,
    tool_calls: &mut Vec<ToolCall>,
) -> Result<(), GenAiInputError> {
    if let Some(part_text) = fields.text.as_deref() {
        text.push_str(part_text);
        return Ok(());
    }
    let Some(function_call) = &fields.function_call else {
        return Ok(());
    };
    tool_calls.push(tool_call(function_call)?);
    Ok(())
}

fn tool_call(call: &Call) -> Result<ToolCall, GenAiInputError> {
    let name = call.name.clone().unwrap_or_default();
    let id = format!("call_{}", call.name.as_deref().unwrap_or("unknown"));
    Ok(ToolCall {
        id,
        kind: String::from("function"),
        function: FunctionCall {
            name,
            arguments: encode_json(call.args.as_ref())?,
        },
    })
}

use crate::oci_chat_wire::tool_schema::resolved_tool_schema;
use crate::oci_chat_wire::types::cohere::{
    CohereMessage, CohereParameterDefinition, CohereTool, CohereToolCall, CohereToolResult,
};
use crate::oci_common::OciError;
use crate::oci_common::schema::{OCI_JSON_TO_PYTHON_TYPES, enrich_cohere_description};
use betterlite_openai::chat::{ChatMessage, MessageContent, Role, Tool, ToolCall};
use betterlite_prompts::arguments::json::{Json, Object};
use std::collections::BTreeMap;
#[must_use]
pub fn extract_text_content(content: Option<&MessageContent>) -> String {
    match content {
        None => String::new(),
        Some(MessageContent::Text(text)) => text.clone(),
        Some(MessageContent::Parts(parts)) => {
            let mut text = String::new();
            for part in parts {
                if part.kind == "text"
                    && let Some(fragment) = part.fields.get("text").and_then(|value| value.as_str())
                {
                    text.push_str(fragment);
                }
            }
            text
        }
    }
}
#[must_use]
pub fn adapt_cohere_messages(messages: &[ChatMessage]) -> Vec<CohereMessage> {
    let lookup = tool_call_lookup(messages);
    let last_user = messages
        .iter()
        .rposition(|message| message.role == Role::User);
    let mut history = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        if Some(index) == last_user {
            continue;
        }
        push_history_message(&mut history, message, &lookup);
    }
    history
}
pub fn adapt_cohere_tools(tools: &[Tool]) -> Result<Vec<CohereTool>, OciError> {
    let mut adapted = Vec::with_capacity(tools.len());
    for tool in tools {
        let definition = tool.function.as_ref();
        let resolved = resolved_tool_schema(definition)?;
        adapted.push(CohereTool {
            name: definition.map_or_else(String::new, |function| function.name.clone()),
            description: definition
                .and_then(|function| function.description.clone())
                .unwrap_or_default(),
            parameter_definitions: parameter_definitions(&resolved),
        });
    }
    Ok(adapted)
}
fn tool_call_lookup(messages: &[ChatMessage]) -> BTreeMap<&str, CohereToolCall> {
    let mut lookup = BTreeMap::new();
    for message in messages {
        if message.role == Role::Assistant
            && let Some(tool_calls) = &message.tool_calls
        {
            for tool_call in tool_calls {
                lookup.insert(tool_call.id.as_str(), cohere_tool_call(tool_call));
            }
        }
    }
    lookup
}
fn push_history_message(
    history: &mut Vec<CohereMessage>,
    message: &ChatMessage,
    lookup: &BTreeMap<&str, CohereToolCall>,
) {
    match message.role {
        Role::User => history.push(CohereMessage::User {
            message: Some(extract_text_content(message.content.as_ref())),
        }),
        Role::Assistant => {
            let tool_calls = message
                .tool_calls
                .as_ref()
                .filter(|calls| !calls.is_empty())
                .map(|calls| calls.iter().map(cohere_tool_call).collect());
            history.push(CohereMessage::Chatbot {
                message: Some(extract_text_content(message.content.as_ref())),
                tool_calls,
            });
        }
        Role::Tool => push_tool_result(history, message, lookup),
        Role::System | Role::Developer | Role::Function => {}
    }
}
fn cohere_tool_call(tool_call: &ToolCall) -> CohereToolCall {
    CohereToolCall {
        name: tool_call.function.name.clone(),
        parameters: serde_json::from_str::<Object>(&tool_call.function.arguments)
            .unwrap_or_default(),
    }
}
fn push_tool_result(
    history: &mut Vec<CohereMessage>,
    message: &ChatMessage,
    lookup: &BTreeMap<&str, CohereToolCall>,
) {
    let call = message
        .tool_call_id
        .as_deref()
        .and_then(|id| lookup.get(id))
        .cloned()
        .unwrap_or_else(|| CohereToolCall {
            name: String::new(),
            parameters: Object::new(),
        });
    let result = CohereToolResult {
        call,
        outputs: vec![Object::from([(
            String::from("output"),
            Json::Text(extract_text_content(message.content.as_ref())),
        )])],
    };
    if let Some(CohereMessage::Tool {
        tool_results: Some(results),
        ..
    }) = history.last_mut()
    {
        results.push(result);
        return;
    }
    history.push(CohereMessage::Tool {
        tool_results: Some(vec![result]),
    });
}
fn parameter_definitions(resolved: &Json) -> BTreeMap<String, CohereParameterDefinition> {
    let Some(object) = resolved.as_object() else {
        return BTreeMap::new();
    };
    let Some(properties) = object.get("properties").and_then(Json::as_object) else {
        return BTreeMap::new();
    };
    let required: &[Json] = match object.get("required") {
        Some(Json::List(names)) => names,
        _ => &[],
    };
    properties
        .iter()
        .map(|(property_name, property_schema)| {
            (
                property_name.clone(),
                parameter_definition(property_name, property_schema, required),
            )
        })
        .collect()
}
fn parameter_definition(
    name: &str,
    property_schema: &Json,
    required: &[Json],
) -> CohereParameterDefinition {
    let fields = property_schema.as_object();
    let description = fields
        .and_then(|schema_fields| schema_fields.get("description"))
        .and_then(Json::as_str)
        .unwrap_or_default();
    let kind = fields
        .and_then(|schema_fields| schema_fields.get("type"))
        .and_then(Json::as_str)
        .filter(|kind| !kind.is_empty())
        .unwrap_or("string");
    let mapped = OCI_JSON_TO_PYTHON_TYPES
        .iter()
        .find(|(json_type, _)| *json_type == kind)
        .map_or(kind, |(_, python_type)| *python_type);
    let empty = Object::new();
    CohereParameterDefinition {
        description: enrich_cohere_description(description, fields.unwrap_or(&empty)),
        kind: mapped.to_owned(),
        is_required: required
            .iter()
            .any(|entry| matches!(entry, Json::Text(text) if text == name)),
    }
}

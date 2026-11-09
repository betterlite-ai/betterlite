use std::collections::BTreeMap;

use betterlite_openai::chat::{ChatMessage, MessageContent, Role, ToolCall};
use betterlite_prompts::arguments::json::{Json, Object};

use crate::oci_chat_wire::types::cohere::{CohereMessage, CohereToolCall, CohereToolResult};

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

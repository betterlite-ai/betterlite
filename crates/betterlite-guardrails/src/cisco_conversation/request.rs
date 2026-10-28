use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::{
    Fields, InspectMessage, Role, caller_role, normalize_content, push_part_text,
};

#[must_use]
pub fn request_messages(data: &Fields) -> Vec<InspectMessage> {
    let mut messages = Vec::new();
    let instructions = normalize_content(&Json::field(data, "instructions"));
    if !instructions.is_empty() {
        messages.push(InspectMessage {
            role: Role::System,
            content: instructions,
        });
    }
    for message in Json::field(data, "messages").objects() {
        messages.extend(chat_message(&message));
    }
    if data.contains_key("input") {
        messages.extend(flatten_input(&Json::field(data, "input")));
    }
    let prompt = Json::field(data, "prompt");
    let prompt_text = normalize_content(&prompt);
    if messages.is_empty() && !prompt_text.is_empty() {
        messages.push(InspectMessage {
            role: Role::User,
            content: prompt_text,
        });
    }
    let tool_text = tool_definition_text(data);
    if !tool_text.is_empty() {
        messages.push(InspectMessage {
            role: Role::System,
            content: tool_text,
        });
    }
    messages
}

fn chat_message(message: &Fields) -> Option<InspectMessage> {
    let role = caller_role(message)?;
    let mut parts = Vec::new();
    let text = normalize_content(&Json::field(message, "content"));
    if !text.is_empty() {
        parts.push(text);
    }
    parts.extend(tool_argument_parts(message));
    (!parts.is_empty()).then(|| InspectMessage {
        role,
        content: parts.join(" "),
    })
}

#[must_use]
pub fn flatten_input(input: &Json) -> Vec<InspectMessage> {
    let mut messages = Vec::new();
    match input {
        Json::Null => {}
        Json::Text(text) => messages.push(InspectMessage {
            role: Role::User,
            content: text.clone(),
        }),
        Json::List(items)
            if items.iter().any(|item| {
                matches!(Json::parse(item.get()), Json::Object(fields) if fields.contains_key("role"))
            }) =>
        {
            for item in input.objects() {
                let mut parts = Vec::new();
                push_part_text(&item, &mut parts);
                let content = parts.join(" ");
                if !content.is_empty() {
                    let role = caller_role(&item).unwrap_or(Role::User);
                    messages.push(InspectMessage { role, content });
                }
            }
        }
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            let content = normalize_content(input);
            if !content.is_empty() {
                messages.push(InspectMessage {
                    role: Role::User,
                    content,
                });
            }
        }
    }
    messages
}

#[must_use]
pub fn tool_argument_parts(message: &Fields) -> Vec<String> {
    let mut parts = Vec::new();
    for call in Json::field(message, "tool_calls").objects() {
        push_arguments(&Json::field(&call, "function"), &mut parts);
    }
    push_arguments(&Json::field(message, "function_call"), &mut parts);
    parts
}

fn push_arguments(function: &Json, parts: &mut Vec<String>) {
    let Json::Object(fields) = function else {
        return;
    };
    let Some(raw) = fields.get("arguments") else {
        return;
    };
    let arguments = Json::parse(raw.get());
    if !matches!(arguments, Json::Null) {
        let text = arguments.python_str();
        if !text.is_empty() {
            parts.push(text);
        }
    }
}

#[must_use]
pub fn tool_definition_text(data: &Fields) -> String {
    let mut parts = Vec::new();
    for key in ["tools", "functions"] {
        collect_strings(&Json::field(data, key), &mut parts);
    }
    parts.join(" ")
}

fn collect_strings(value: &Json, parts: &mut Vec<String>) {
    match value {
        Json::Text(text) if !text.is_empty() => parts.push(text.clone()),
        Json::List(items) => {
            for item in items {
                collect_strings(&Json::parse(item.get()), parts);
            }
        }
        Json::Object(fields) => {
            for item in fields.values() {
                collect_strings(&Json::parse(item.get()), parts);
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}

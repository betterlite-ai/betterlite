use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{AllMessageValues, PromptParams};
use indexmap::IndexMap;

use crate::langfuse_rest::records::PromptMessage;

use super::LangfusePromptManagementError;

const NOT_GIVEN: &str = "NOT_GIVEN";

// langfuse.model.TemplateParser.compile_template: unresolved variables stay verbatim, values become text
#[must_use]
pub(crate) fn compile_template(content: &str, variables: &PromptParams) -> String {
    let mut compiled = String::new();
    let mut rest = content;
    loop {
        let Some((before, after_open)) = rest.split_once("{{") else {
            compiled.push_str(rest);
            return compiled;
        };
        compiled.push_str(before);
        let Some((name, after_close)) = after_open.split_once("}}") else {
            compiled.push_str("{{");
            compiled.push_str(after_open);
            return compiled;
        };
        match variables.get(name.trim()) {
            Some(value) => compiled.push_str(&compile_value(value)),
            None => {
                compiled.push_str("{{");
                compiled.push_str(name);
                compiled.push_str("}}");
            }
        }
        rest = after_close;
    }
}

// str(v) in Python, except that a list or dict variable is substituted as its JSON text
fn compile_value(value: &Json) -> String {
    match value {
        Json::Null => String::new(),
        Json::Bool(set) => set.to_string(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

// langfuse.model.ChatPromptClient.compile: chat messages keep their order and each placeholder expands
pub(crate) fn compile_chat_prompt(
    messages: &[PromptMessage],
    variables: &PromptParams,
) -> Result<Vec<AllMessageValues>, LangfusePromptManagementError> {
    let mut compiled = Vec::with_capacity(messages.len());
    let mut unresolved = Vec::new();
    for message in messages {
        match message {
            PromptMessage::Chat(chat) => compiled.push(message_object(
                &chat.role,
                &compile_template(&chat.content, variables),
            )),
            PromptMessage::Placeholder(placeholder) => {
                compile_placeholder(&placeholder.name, variables, &mut compiled, &mut unresolved)?
            }
        }
    }
    if !unresolved.is_empty() {
        tracing::warn!(
            "Placeholders {unresolved:?} have not been resolved. Pass them as variables to compile()."
        );
    }
    Ok(compiled)
}

// TemplateParser: a placeholder with no variable stays a placeholder message so the caller can fill it
fn compile_placeholder(
    name: &str,
    variables: &PromptParams,
    compiled: &mut Vec<AllMessageValues>,
    unresolved: &mut Vec<String>,
) -> Result<(), LangfusePromptManagementError> {
    let Some(value) = variables.get(name) else {
        compiled.push(placeholder_object(name));
        unresolved.push(name.to_owned());
        return Ok(());
    };
    let Json::List(items) = value else {
        tracing::warn!("Placeholder '{name}' must contain a list of chat messages");
        compiled.push(message_object(NOT_GIVEN, &compile_value(value)));
        return Ok(());
    };
    for item in items {
        match item {
            Json::Object(fields) => {
                compiled.push(compile_placeholder_message(fields, variables));
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                tracing::warn!(
                    "Placeholder '{name}' should contain a list of chat messages with 'role' and 'content' fields. Appended as string."
                );
                compiled.push(message_object(NOT_GIVEN, &compile_value(value)));
            }
        }
    }
    Ok(())
}

// a resolved placeholder message keeps its own fields, with role defaulting to NOT_GIVEN
fn compile_placeholder_message(
    fields: &IndexMap<String, Json>,
    variables: &PromptParams,
) -> AllMessageValues {
    let content = match fields.get("content") {
        None | Some(Json::Null) => String::new(),
        Some(Json::Text(text)) => text.clone(),
        Some(value @ (Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))) => {
            compile_value(value)
        }
    };
    let mut message = fields.clone();
    let role = fields
        .get("role")
        .cloned()
        .unwrap_or_else(|| Json::Text(NOT_GIVEN.to_owned()));
    message.insert("role".to_owned(), role);
    message.insert(
        "content".to_owned(),
        Json::Text(compile_template(&content, variables)),
    );
    Json::Object(message)
}

fn message_object(role: &str, content: &str) -> AllMessageValues {
    Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text(role.to_owned())),
        ("content".to_owned(), Json::Text(content.to_owned())),
    ]))
}

fn placeholder_object(name: &str) -> AllMessageValues {
    Json::Object(IndexMap::from([
        ("type".to_owned(), Json::Text("placeholder".to_owned())),
        ("name".to_owned(), Json::Text(name.to_owned())),
    ]))
}

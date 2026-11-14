use std::collections::{BTreeMap, BTreeSet};

use betterlite_prompts::arguments::json::Object as PromptObject;
use serde_json::Value;

use super::error::HostedVllmChatError;
use super::video::{convert_file_to_video_url, is_video_file};
use crate::chat::{ChatMessage, ContentPart, FunctionCall, MessageContent, Role, ToolCall};
use crate::chat_content::{ChatContent, Mode, UrlDownloader};
use crate::openai_base_config::Object;

const FILE: &str = "file";
const FUNCTION: &str = "function";
const ID: &str = "id";
const INPUT: &str = "input";
const NAME: &str = "name";
const PROVIDER: &str = "hosted_vllm";
const REASONING_CONTENT: &str = "reasoning_content";
const TEXT: &str = "text";
const THINKING_BLOCKS: &str = "thinking_blocks";
const TOOL_USE: &str = "tool_use";

fn transform_assistant(message: &mut ChatMessage) -> Result<(), HostedVllmChatError> {
    message.passthrough.shift_remove(THINKING_BLOCKS);
    if !matches!(
        message.passthrough.get(REASONING_CONTENT),
        Some(Value::String(_))
    ) {
        message.passthrough.shift_remove(REASONING_CONTENT);
    }
    let Some(MessageContent::Parts(items)) = &message.content else {
        return Ok(());
    };
    let mut text_parts: Vec<String> = Vec::new();
    let mut calls: Vec<ToolCall> = Vec::new();
    let mut blocks: Vec<ContentPart> = Vec::with_capacity(items.len());
    let mut structured = false;
    for item in items {
        match item.kind.as_str() {
            TEXT => {
                match item.fields.get(TEXT) {
                    None | Some(Value::Null) => text_parts.push(String::new()),
                    Some(Value::String(text)) => text_parts.push(text.clone()),
                    Some(
                        Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_),
                    ) => {
                        return Err(HostedVllmChatError::PartString {
                            part: TEXT,
                            field: TEXT,
                        });
                    }
                }
                blocks.push(item.clone());
            }
            TOOL_USE => {
                let id = match item.fields.get(ID) {
                    Some(Value::String(id)) => id.clone(),
                    None
                    | Some(
                        Value::Null
                        | Value::Bool(_)
                        | Value::Number(_)
                        | Value::Array(_)
                        | Value::Object(_),
                    ) => {
                        return Err(HostedVllmChatError::PartString {
                            part: TOOL_USE,
                            field: ID,
                        });
                    }
                };
                let name = match item.fields.get(NAME) {
                    Some(Value::String(name)) => name.clone(),
                    None
                    | Some(
                        Value::Null
                        | Value::Bool(_)
                        | Value::Number(_)
                        | Value::Array(_)
                        | Value::Object(_),
                    ) => {
                        return Err(HostedVllmChatError::PartString {
                            part: TOOL_USE,
                            field: NAME,
                        });
                    }
                };
                let given = item
                    .fields
                    .get(INPUT)
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Object::new()));
                let arguments = match given {
                    Value::String(text) => text,
                    other => serde_json::to_string(&other)
                        .map_err(|source| HostedVllmChatError::Json { source })?,
                };
                calls.push(ToolCall {
                    id,
                    kind: String::from(FUNCTION),
                    function: FunctionCall { name, arguments },
                });
            }
            _ => {
                blocks.push(item.clone());
                structured = true;
            }
        }
    }
    if !calls.is_empty() {
        match message.tool_calls.take() {
            Some(mut existing) => {
                let ids: BTreeSet<String> = existing.iter().map(|call| call.id.clone()).collect();
                let added: Vec<ToolCall> = calls
                    .into_iter()
                    .filter(|call| !ids.contains(&call.id))
                    .collect();
                existing.extend(added);
                message.tool_calls = Some(existing);
            }
            None => message.tool_calls = Some(calls),
        }
    }
    message.content = Some(if structured {
        MessageContent::Parts(blocks)
    } else {
        MessageContent::Text(text_parts.join("\n"))
    });
    Ok(())
}

fn transform_user(message: &mut ChatMessage) -> Result<(), HostedVllmChatError> {
    let Some(MessageContent::Parts(items)) = &message.content else {
        return Ok(());
    };
    let mut converted = Vec::with_capacity(items.len());
    for item in items {
        if item.kind == FILE && is_video_file(item)? {
            converted.push(convert_file_to_video_url(item)?);
        } else {
            converted.push(item.clone());
        }
    }
    message.content = Some(MessageContent::Parts(converted));
    Ok(())
}

pub fn transform_messages(
    model: &str,
    messages: &[ChatMessage],
    downloads: &dyn UrlDownloader,
) -> Result<Vec<ChatMessage>, HostedVllmChatError> {
    let mut transformed = Vec::with_capacity(messages.len());
    for message in messages {
        let mut converted = message.clone();
        match converted.role {
            Role::Assistant => transform_assistant(&mut converted)?,
            Role::User => transform_user(&mut converted)?,
            Role::System | Role::Developer | Role::Tool | Role::Function => {}
        }
        transformed.push(converted);
    }
    let environment = BTreeMap::new();
    let content = ChatContent {
        model,
        messages: &transformed,
        optional_params: &PromptObject::new(),
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: &environment,
        downloads,
    };
    Ok(content.transform(Mode::Sync)?.messages)
}

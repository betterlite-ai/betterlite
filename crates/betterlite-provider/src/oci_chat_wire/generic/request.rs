use crate::oci_chat_wire::tool_schema::resolved_tool_schema;
use crate::oci_chat_wire::types::oci::{
    OciContentPart, OciFunctionType, OciImagePart, OciImageType, OciImageUrl, OciMessage, OciRole,
    OciTextPart, OciTextType, OciToolCall, OciToolDefinition,
};
use crate::oci_common::OciError;
use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent, Role, Tool, ToolCall};
const TEXT_FIELD: &str = "Content item of type `text` must have a string `text` field";
const IMAGE_URL_FIELD: &str =
    "Prop `image_url` must be a string or an object with a `url` property";
const TOOL_CALL_KIND: &str = "OCI only supports function tool calls";
const TOOL_KIND: &str = "OCI only supports function tools";
const FUNCTION_OBJECT: &str = "Tool `function` must be a dictionary";
const TOOL_RESULT_ID: &str = "Tool result message must have a string `tool_call_id`";
const TOOL_RESULT_CONTENT: &str = "Tool result message `content` must be a string";
pub fn adapt_generic_messages(messages: &[ChatMessage]) -> Result<Vec<OciMessage>, OciError> {
    let mut adapted = Vec::with_capacity(messages.len());
    for message in messages {
        match message.role {
            Role::Assistant if message.tool_calls.is_some() => {
                adapted.push(tool_call_message(
                    message.tool_calls.as_deref().unwrap_or_default(),
                )?);
            }
            Role::System => push_content(&mut adapted, OciRole::System, message)?,
            Role::User => push_content(&mut adapted, OciRole::User, message)?,
            Role::Assistant => push_content(&mut adapted, OciRole::Assistant, message)?,
            Role::Tool => adapted.push(tool_response(message)?),
            Role::Developer | Role::Function => {}
        }
    }
    Ok(adapted)
}
pub fn adapt_generic_tools(tools: &[Tool]) -> Result<Vec<OciToolDefinition>, OciError> {
    let mut definitions = Vec::with_capacity(tools.len());
    for tool in tools {
        if tool.kind != "function" {
            return Err(OciError {
                status: 400,
                message: String::from(TOOL_KIND),
            });
        }
        let Some(function) = &tool.function else {
            return Err(OciError {
                status: 400,
                message: String::from(FUNCTION_OBJECT),
            });
        };
        definitions.push(OciToolDefinition {
            kind: OciFunctionType::Function,
            name: Some(function.name.clone()),
            description: Some(function.description.clone().unwrap_or_default()),
            parameters: Some(resolved_tool_schema(Some(function))?),
        });
    }
    Ok(definitions)
}
fn push_content(
    adapted: &mut Vec<OciMessage>,
    role: OciRole,
    message: &ChatMessage,
) -> Result<(), OciError> {
    if let Some(content) = &message.content {
        adapted.push(content_message(role, content)?);
    }
    Ok(())
}
fn content_message(role: OciRole, content: &MessageContent) -> Result<OciMessage, OciError> {
    let parts = match content {
        MessageContent::Text(text) => vec![OciContentPart::Text(OciTextPart {
            kind: OciTextType::Text,
            text: text.clone(),
        })],
        MessageContent::Parts(parts) => parts
            .iter()
            .map(content_part)
            .collect::<Result<Vec<OciContentPart>, OciError>>()?,
    };
    Ok(OciMessage {
        role,
        content: Some(parts),
        tool_calls: None,
        tool_call_id: None,
    })
}
fn image_url(part: &ContentPart) -> Option<&str> {
    let image = part.fields.get("image_url")?;
    if let Some(url) = image.as_str() {
        return Some(url);
    }
    image.get("url")?.as_str()
}
fn content_part(part: &ContentPart) -> Result<OciContentPart, OciError> {
    match part.kind.as_str() {
        "text" => {
            let Some(text) = part.fields.get("text").and_then(|value| value.as_str()) else {
                return Err(OciError {
                    status: 400,
                    message: String::from(TEXT_FIELD),
                });
            };
            Ok(OciContentPart::Text(OciTextPart {
                kind: OciTextType::Text,
                text: text.to_owned(),
            }))
        }
        "image_url" => {
            let Some(url) = image_url(part) else {
                return Err(OciError {
                    status: 400,
                    message: String::from(IMAGE_URL_FIELD),
                });
            };
            Ok(OciContentPart::Image(OciImagePart {
                kind: OciImageType::Image,
                image_url: OciImageUrl {
                    url: url.to_owned(),
                    detail: None,
                },
            }))
        }
        other => Err(OciError {
            status: 400,
            message: format!("Content type `{other}` is not supported by OCI"),
        }),
    }
}
fn tool_call_message(tool_calls: &[ToolCall]) -> Result<OciMessage, OciError> {
    let mut adapted = Vec::with_capacity(tool_calls.len());
    for tool_call in tool_calls {
        if tool_call.kind != "function" {
            return Err(OciError {
                status: 400,
                message: String::from(TOOL_CALL_KIND),
            });
        }
        adapted.push(OciToolCall {
            id: Some(tool_call.id.clone()),
            kind: OciFunctionType::Function,
            name: tool_call.function.name.clone(),
            arguments: tool_call.function.arguments.clone(),
        });
    }
    Ok(OciMessage {
        role: OciRole::Assistant,
        content: None,
        tool_calls: Some(adapted),
        tool_call_id: None,
    })
}
fn tool_response(message: &ChatMessage) -> Result<OciMessage, OciError> {
    let Some(tool_call_id) = &message.tool_call_id else {
        return Err(OciError {
            status: 400,
            message: String::from(TOOL_RESULT_ID),
        });
    };
    let Some(MessageContent::Text(content)) = &message.content else {
        return Err(OciError {
            status: 400,
            message: String::from(TOOL_RESULT_CONTENT),
        });
    };
    Ok(OciMessage {
        role: OciRole::Tool,
        content: Some(vec![OciContentPart::Text(OciTextPart {
            kind: OciTextType::Text,
            text: content.clone(),
        })]),
        tool_calls: None,
        tool_call_id: Some(tool_call_id.clone()),
    })
}

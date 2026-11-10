use serde_json::Value;

use crate::chat::ChatMessage;
use crate::gigachat_chat::config::GigachatImageUploader;
use crate::gigachat_chat::error::GigaChatError;
use crate::openai_base_config::Object;

pub async fn transform_messages(
    messages: &[ChatMessage],
    uploader: &dyn GigachatImageUploader,
) -> Result<Vec<Value>, GigaChatError> {
    let mut transformed = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        transformed.push(transform_message(index, message, uploader).await?);
    }
    Ok(transformed)
}

async fn transform_message(
    index: usize,
    message: &ChatMessage,
    uploader: &dyn GigachatImageUploader,
) -> Result<Value, GigaChatError> {
    let value = serde_json::to_value(message).map_err(|source| GigaChatError {
        status_code: 500,
        message: format!("the GigaChat request message cannot be encoded: {source}"),
        headers: None,
    })?;
    let Value::Object(mut object) = value else {
        return Err(GigaChatError {
            status_code: 500,
            message: String::from("the GigaChat request message is not an object"),
            headers: None,
        });
    };
    object.remove("name");
    rewrite_role(index, &mut object)?;
    rewrite_content(&mut object, uploader).await?;
    rewrite_tool_calls(message, &mut object);
    Ok(Value::Object(object))
}

// GigaChat takes a system message only at the first position, a developer role at none, and a tool role not at all
fn rewrite_role(index: usize, object: &mut Object) -> Result<(), GigaChatError> {
    let role = object
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    match role.as_str() {
        "developer" => {
            object.insert(String::from("role"), Value::String(String::from("system")));
        }
        "system" if index > 0 => {
            object.insert(String::from("role"), Value::String(String::from("user")));
        }
        "tool" => {
            object.insert(
                String::from("role"),
                Value::String(String::from("function")),
            );
            let content = object
                .get("content")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            let encoded = tool_content(&content)?;
            object.insert(String::from("content"), encoded);
        }
        _ => {}
    }
    Ok(())
}

fn tool_content(content: &Value) -> Result<Value, GigaChatError> {
    if let Value::String(text) = content
        && serde_json::from_str::<Value>(text).is_ok()
    {
        return Ok(Value::String(text.clone()));
    }
    serde_json::to_string(content)
        .map(Value::String)
        .map_err(|source| GigaChatError {
            status_code: 500,
            message: format!("the GigaChat tool message content cannot be encoded: {source}"),
            headers: None,
        })
}

async fn rewrite_content(
    object: &mut Object,
    uploader: &dyn GigachatImageUploader,
) -> Result<(), GigaChatError> {
    if object.get("content").is_none_or(Value::is_null) {
        object.insert(String::from("content"), Value::String(String::new()));
    }
    let Some(Value::Array(parts)) = object.get("content").cloned() else {
        return Ok(());
    };
    let (text, attachments) = transform_list_content(&parts, uploader).await?;
    object.insert(String::from("content"), Value::String(text));
    if !attachments.is_empty() {
        object.insert(
            String::from("attachments"),
            Value::Array(attachments.into_iter().map(Value::String).collect()),
        );
    }
    Ok(())
}

fn rewrite_tool_calls(message: &ChatMessage, object: &mut Object) {
    let Some(first) = message
        .tool_calls
        .as_ref()
        .and_then(|tool_calls| tool_calls.first())
    else {
        return;
    };
    let arguments = serde_json::from_str::<Value>(&first.function.arguments)
        .unwrap_or_else(|_| Value::Object(Object::new()));
    object.insert(
        String::from("function_call"),
        Value::Object(Object::from_iter([
            (
                String::from("name"),
                Value::String(first.function.name.clone()),
            ),
            (String::from("arguments"), arguments),
        ])),
    );
    object.remove("tool_calls");
}

async fn transform_list_content(
    parts: &[Value],
    uploader: &dyn GigachatImageUploader,
) -> Result<(String, Vec<String>), GigaChatError> {
    let mut texts = Vec::new();
    let mut attachments = Vec::new();
    for entry in parts {
        let Some(part) = entry.as_object() else {
            continue;
        };
        match part.get("type").and_then(Value::as_str) {
            Some("text") => texts.push(text_of(part)?),
            Some("image_url") => {
                let url = image_url_of(part)?;
                if !url.is_empty()
                    && let Some(file_id) = uploader.upload(&url).await
                {
                    attachments.push(file_id);
                }
            }
            None | Some(_) => {}
        }
    }
    Ok((texts.join("\n"), attachments))
}

fn text_of(part: &Object) -> Result<String, GigaChatError> {
    match part.get("text") {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            Err(GigaChatError {
                status_code: 400,
                message: String::from("a text content part must hold a string text"),
                headers: None,
            })
        }
    }
}

fn image_url_of(part: &Object) -> Result<String, GigaChatError> {
    match part.get("image_url") {
        Some(Value::String(url)) => Ok(url.clone()),
        Some(Value::Object(image)) => match image.get("url") {
            None | Some(Value::Null) => Ok(String::new()),
            Some(Value::String(url)) => Ok(url.clone()),
            Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
                Err(GigaChatError {
                    status_code: 400,
                    message: String::from("an image_url part must hold a string url"),
                    headers: None,
                })
            }
        },
        None | Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_)) => {
            Ok(String::new())
        }
    }
}

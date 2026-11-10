use serde::Deserialize;
use serde_json::Value;

use crate::chat::ChatMessage;
use crate::gigachat_chat::config::GigachatImageUploader;
use crate::gigachat_chat::error::GigaChatError;
use crate::openai_base_config::Object;

#[derive(Deserialize)]
struct ContentPart {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    image_url: Option<ImageUrl>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ImageUrl {
    Link(String),
    Fields { url: Option<String> },
}

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
    let mut object: Object = serde_json::to_value(message)
        .and_then(serde_json::from_value)
        .map_err(|source| {
            GigaChatError::server_error(format!(
                "the GigaChat request message cannot be encoded: {source}"
            ))
        })?;
    object.remove("name");
    rewrite_role(index, &mut object);
    rewrite_content(&mut object, uploader).await?;
    rewrite_tool_calls(message, &mut object);
    Ok(Value::Object(object))
}

// GigaChat takes a system message only at the first position, a developer role at none, and a tool role not at all
fn rewrite_role(index: usize, object: &mut Object) {
    let Some(role) = object
        .get("role")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    let renamed = match role.as_str() {
        "developer" => "system",
        "system" if index > 0 => "user",
        "tool" => "function",
        _ => return,
    };
    if role == "tool" {
        let content = object
            .get("content")
            .cloned()
            .unwrap_or_else(|| Value::from(""));
        object.insert("content".to_owned(), tool_content(&content));
    }
    object.insert("role".to_owned(), Value::from(renamed));
}

// a function message holds JSON text, so anything that is not JSON text yet is written as JSON
fn tool_content(content: &Value) -> Value {
    if let Value::String(text) = content
        && serde_json::from_str::<Value>(text).is_ok()
    {
        return content.clone();
    }
    Value::String(content.to_string())
}

async fn rewrite_content(
    object: &mut Object,
    uploader: &dyn GigachatImageUploader,
) -> Result<(), GigaChatError> {
    if object.get("content").is_none_or(Value::is_null) {
        object.insert("content".to_owned(), Value::from(""));
    }
    let Some(Value::Array(parts)) = object.get("content").cloned() else {
        return Ok(());
    };
    let mut texts = Vec::new();
    let mut attachments = Vec::new();
    for entry in parts.into_iter().filter(Value::is_object) {
        let part: ContentPart = serde_json::from_value(entry).map_err(|source| {
            GigaChatError::bad_request(&format!("a content part holds declared fields: {source}"))
        })?;
        match (part.kind.as_deref(), part.image_url) {
            (Some("text"), _) => texts.push(part.text.unwrap_or_default()),
            (
                Some("image_url"),
                Some(ImageUrl::Link(url) | ImageUrl::Fields { url: Some(url) }),
            ) => {
                if !url.is_empty()
                    && let Some(file_id) = uploader.upload(&url).await
                {
                    attachments.push(Value::String(file_id));
                }
            }
            _ => {}
        }
    }
    object.insert("content".to_owned(), Value::String(texts.join("\n")));
    if !attachments.is_empty() {
        object.insert("attachments".to_owned(), Value::Array(attachments));
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
        "function_call".to_owned(),
        Value::Object(Object::from_iter([
            ("name".to_owned(), Value::from(first.function.name.as_str())),
            ("arguments".to_owned(), arguments),
        ])),
    );
    object.remove("tool_calls");
}

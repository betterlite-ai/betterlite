use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use thiserror::Error;

mod files;
mod image;
pub(crate) mod tools;

#[derive(Debug, Error)]
pub enum MessageError {
    #[error(transparent)]
    Image(#[from] betterlite_prompts::anthropic_results::images::ImageError),
    #[error(transparent)]
    File(#[from] betterlite_prompts::anthropic_invokes::documents::FileMessageError),
    #[error(transparent)]
    ToolResult(
        #[from]
        betterlite_prompts::anthropic_results::tool_result::ToolResultError<files::FileBlockError>,
    ),
    #[error(transparent)]
    FileBlock(#[from] files::FileBlockError),
    #[error("message field {field} must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitMessages {
    pub system: Option<Vec<Json>>,
    pub conversation: Vec<Json>,
}

pub async fn extract_system_and_messages<Source>(
    messages: &[Message],
    fetcher: &Source,
) -> Result<SplitMessages, MessageError>
where
    Source: betterlite_prompts::media::inline::RemoteMediaFetcher + Sync,
{
    let mut system_parts: Vec<Json> = Vec::new();
    let mut conversation: Vec<Json> = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "system" => collect_system(message, &mut system_parts),
            "assistant" => assistant_turn(message, &mut conversation)?,
            "tool" => tool_turn(message, fetcher, &mut conversation).await?,
            role => conversation.push(other_turn(role, message, fetcher).await?),
        }
    }
    let system = (!system_parts.is_empty()).then_some(system_parts);
    Ok(SplitMessages {
        system,
        conversation,
    })
}

fn collect_system(message: &Message, system_parts: &mut Vec<Json>) {
    match message.content.as_ref() {
        Some(Content::Text(text)) if !text.is_empty() => {
            system_parts.push(Json::Object(text_block(Json::Text(text.clone()))));
        }
        Some(Content::Parts(parts)) => {
            for part in parts.iter().filter(|part| part.kind == "text") {
                let text = part
                    .fields
                    .get("text")
                    .cloned()
                    .unwrap_or_else(|| Json::Text(String::new()));
                let mut block = text_block(text);
                if let Some(cache_control) = part.fields.get("cache_control") {
                    block.insert("cache_control".to_owned(), cache_control.clone());
                }
                system_parts.push(Json::Object(block));
            }
        }
        Some(Content::Text(_)) | None => {}
    }
}

fn text_block(text: Json) -> Object {
    Object::from_iter([
        ("type".to_owned(), Json::Text("text".to_owned())),
        ("text".to_owned(), text),
    ])
}

fn signed_thinking_blocks(message: &Message) -> Vec<Json> {
    let Some(Json::List(blocks)) = message.provider_specific_fields.get("thinking_blocks") else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|block| {
            let Some(fields) = block.as_object() else {
                return false;
            };
            let signed = fields.get("signature").is_some_and(Json::is_truthy);
            let redacted = fields.get("type").and_then(Json::as_str) == Some("redacted_thinking");
            signed || redacted
        })
        .cloned()
        .collect()
}

fn assistant_turn(message: &Message, conversation: &mut Vec<Json>) -> Result<(), MessageError> {
    let content = content_json(message.content.as_ref());
    let thinking_blocks = signed_thinking_blocks(message);
    if let Some(Json::List(calls)) = message.provider_specific_fields.get("tool_calls")
        && !calls.is_empty()
    {
        let blocks = assistant_tool_use_calls(&content, &thinking_blocks, calls)?;
        conversation.push(object_turn("assistant", Json::List(blocks)));
        return Ok(());
    }
    if !thinking_blocks.is_empty() {
        let mut blocks = thinking_blocks;
        match &content {
            Json::List(items) => blocks.extend(items.iter().cloned()),
            other if other.is_truthy() => blocks.push(Json::Object(text_block(other.clone()))),
            _ => {}
        }
        conversation.push(object_turn("assistant", Json::List(blocks)));
        return Ok(());
    }
    conversation.push(object_turn("assistant", content));
    Ok(())
}

fn assistant_tool_use_calls(
    content: &Json,
    thinking_blocks: &[Json],
    calls: &[Json],
) -> Result<Vec<Json>, MessageError> {
    let mut blocks: Vec<Json> = thinking_blocks.to_vec();
    if content.is_truthy() {
        blocks.push(Json::Object(text_block(content.clone())));
    }
    for call in calls {
        let fields = call.as_object().ok_or(MessageError::Field {
            field: "tool_calls",
            expected: "a list of objects",
        })?;
        let function = call_function(fields)?;
        blocks.push(Json::Object(Object::from_iter([
            ("type".to_owned(), Json::Text("tool_use".to_owned())),
            (
                "id".to_owned(),
                fields
                    .get("id")
                    .cloned()
                    .unwrap_or_else(|| Json::Text(String::new())),
            ),
            (
                "name".to_owned(),
                function
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| Json::Text(String::new())),
            ),
            ("input".to_owned(), tool_input(function.get("arguments"))),
        ])));
    }
    Ok(blocks)
}

fn call_function(fields: &Object) -> Result<Object, MessageError> {
    match fields.get("function") {
        None => Ok(Object::new()),
        Some(Json::Object(function)) => Ok(function.clone()),
        Some(_) => Err(MessageError::Field {
            field: "function",
            expected: "an object",
        }),
    }
}

// Python decodes string arguments, keeps every other value as it is, and gives {} for absent ones
fn tool_input(arguments: Option<&Json>) -> Json {
    let Some(value) = arguments else {
        return Json::Object(Object::new());
    };
    let Json::Text(text) = value else {
        return value.clone();
    };
    serde_json::from_str(text).unwrap_or_else(|_| Json::Object(Object::new()))
}

async fn tool_turn<Source>(
    message: &Message,
    fetcher: &Source,
    conversation: &mut Vec<Json>,
) -> Result<(), MessageError>
where
    Source: betterlite_prompts::media::inline::RemoteMediaFetcher + Sync,
{
    let tool_call_id = match message.provider_specific_fields.get("tool_call_id") {
        Some(Json::Text(id)) => id.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            String::new()
        }
    };
    let cache_control = message
        .provider_specific_fields
        .get("cache_control")
        .filter(|value| !matches!(value, Json::Null))
        .cloned();
    let block = match message.content.as_ref() {
        Some(Content::Parts(parts)) => {
            files::tool_result_from_parts(parts, &tool_call_id, cache_control, fetcher).await?
        }
        Some(Content::Text(text)) => plain_tool_result(text.clone(), &tool_call_id, cache_control),
        None => plain_tool_result(String::new(), &tool_call_id, cache_control),
    };
    append_tool_result(conversation, block);
    Ok(())
}

fn plain_tool_result(content: String, tool_call_id: &str, cache_control: Option<Json>) -> Object {
    let mut block = Object::from_iter([
        ("type".to_owned(), Json::Text("tool_result".to_owned())),
        (
            "tool_use_id".to_owned(),
            Json::Text(tool_call_id.to_owned()),
        ),
        ("content".to_owned(), Json::Text(content)),
    ]);
    if let Some(value) = cache_control {
        block.insert("cache_control".to_owned(), value);
    }
    block
}

fn append_tool_result(conversation: &mut Vec<Json>, block: Object) {
    if let Some(Json::Object(last)) = conversation.last_mut()
        && last.get("role").and_then(Json::as_str) == Some("user")
        && let Some(Json::List(blocks)) = last.get_mut("content")
        && !blocks.is_empty()
        && blocks
            .first()
            .and_then(Json::as_object)
            .and_then(|first| first.get("type"))
            .and_then(Json::as_str)
            == Some("tool_result")
    {
        blocks.push(Json::Object(block));
        return;
    }
    conversation.push(object_turn("user", Json::List(vec![Json::Object(block)])));
}

async fn other_turn<Source>(
    role: &str,
    message: &Message,
    fetcher: &Source,
) -> Result<Json, MessageError>
where
    Source: betterlite_prompts::media::inline::RemoteMediaFetcher + Sync,
{
    let content = image::convert_image_url_blocks(message.content.as_ref(), fetcher).await?;
    Ok(object_turn(role, content))
}

fn object_turn(role: &str, content: Json) -> Json {
    Json::Object(Object::from_iter([
        ("role".to_owned(), Json::Text(role.to_owned())),
        ("content".to_owned(), content),
    ]))
}

pub(crate) fn content_json(content: Option<&Content>) -> Json {
    match content {
        None => Json::Text(String::new()),
        Some(Content::Text(text)) => Json::Text(text.clone()),
        Some(Content::Parts(parts)) => Json::List(parts.iter().map(part_json).collect()),
    }
}

pub(crate) fn part_json(part: &ContentPart) -> Json {
    let mut fields = part.fields.clone();
    fields.insert("type".to_owned(), Json::Text(part.kind.clone()));
    Json::Object(fields)
}

#[cfg(test)]
mod tests;

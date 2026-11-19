use betterlite_prompts::arguments::json::{Json, Object};

use super::error::ChatContentError;
use super::{ChatTarget, text_part};
use crate::api::messages_content_primitives::markers::{
    carry_cache_breakpoint, copy_cache_control,
};
use crate::api::messages_content_primitives::tool_results::{
    image_source_url, tool_result_content,
};

fn user_message(content: Json) -> Json {
    Json::Object(Object::from([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), content),
    ]))
}

fn media_part(block: &Object, model: Option<&str>) -> Option<Object> {
    let url = block
        .get("source")
        .and_then(image_source_url)
        .filter(|url| !url.is_empty())?;
    let mut image_url = Object::new();
    image_url.insert("url".to_owned(), Json::Text(url));
    let mut part = Object::new();
    part.insert("type".to_owned(), Json::Text("image_url".to_owned()));
    part.insert("image_url".to_owned(), Json::Object(image_url));
    copy_cache_control(block, &mut part, model);
    Some(part)
}

fn push_tool_result(
    tool_messages: &mut Vec<Json>,
    block: &Object,
    model: Option<&str>,
) -> Result<(), ChatContentError> {
    let id = block
        .get("tool_use_id")
        .and_then(Json::as_str)
        .unwrap_or_default();
    let content = block.get("content").unwrap_or(&Json::Null);
    let mut message = Object::new();
    message.insert("role".to_owned(), Json::Text("tool".to_owned()));
    message.insert("tool_call_id".to_owned(), Json::Text(id.to_owned()));
    let encoded = serde_json::to_value(tool_result_content(content))?;
    message.insert("content".to_owned(), serde_json::from_value(encoded)?);
    copy_cache_control(block, &mut message, model);
    tool_messages.push(Json::Object(message));
    Ok(())
}

fn push_blocks(
    blocks: &[Json],
    target: &ChatTarget<'_>,
    new_messages: &mut Vec<Json>,
) -> Result<(), ChatContentError> {
    let mut tool_messages = Vec::new();
    let mut content_list = Vec::new();
    for block in blocks {
        let Some(fields) = block.as_object() else {
            continue;
        };
        match fields.get("type").and_then(Json::as_str) {
            Some("text") => {
                let mut part = text_part(fields);
                copy_cache_control(fields, &mut part, target.model);
                content_list.push(Json::Object(carry_cache_breakpoint(fields, &part)));
            }
            Some("image") => {
                if let Some(part) = media_part(fields, target.model) {
                    content_list.push(Json::Object(carry_cache_breakpoint(fields, &part)));
                }
            }
            Some("document") => {
                if let Some(part) = media_part(fields, target.model) {
                    content_list.push(Json::Object(part));
                }
            }
            Some("tool_result") => push_tool_result(&mut tool_messages, fields, target.model)?,
            Some(_) | None => {}
        }
    }
    new_messages.extend(tool_messages);
    if !content_list.is_empty() {
        new_messages.push(user_message(Json::List(content_list)));
    }
    Ok(())
}

pub fn push_user_messages(
    fields: &Object,
    target: &ChatTarget<'_>,
    new_messages: &mut Vec<Json>,
) -> Result<(), ChatContentError> {
    match fields.get("content") {
        Some(Json::Text(text)) if !text.is_empty() => {
            new_messages.push(user_message(Json::Text(text.clone())));
        }
        Some(Json::List(blocks)) => push_blocks(blocks, target, new_messages)?,
        Some(_) | None => {}
    }
    Ok(())
}

use crate::arguments::json::{Json, Object};

use super::continuation::assistant_continue_message;
use super::error::ContentError;
use super::messages::{
    CONTENT, ROLE, TEXT, TEXT_TYPE, TYPE, content_list_to_str, has_non_content_values, is_blank,
};

pub fn skip_empty_text_blocks(message: &Json) -> Result<Json, ContentError> {
    let Some(fields) = message.as_object() else {
        return Err(ContentError::MessageNotObject);
    };
    match fields.get(CONTENT) {
        Some(Json::Text(text)) => skip_blank_text_content(message, fields, text),
        Some(Json::List(parts)) => skip_text_block_list(fields, parts),
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {
            Ok(message.clone())
        }
    }
}

pub fn process_empty_text_blocks(
    message: &Json,
    continue_message: Option<&Json>,
) -> Result<Json, ContentError> {
    let Some(fields) = message.as_object() else {
        return Err(ContentError::MessageNotObject);
    };
    let Some(Json::List(parts)) = fields.get(CONTENT) else {
        return Ok(message.clone());
    };
    let mut kept = Vec::with_capacity(parts.len());
    let mut all_empty = true;
    for (index, part) in parts.iter().enumerate() {
        if part_is_empty_text(part, index)? {
            continue;
        }
        all_empty = false;
        kept.push(part.clone());
    }
    let mut modified = fields.clone();
    if all_empty {
        let continue_fields = assistant_continue_message(continue_message);
        let text = content_list_to_str(&continue_fields);
        modified.insert(
            CONTENT.to_owned(),
            Json::List(vec![Json::Object(Object::from_iter([
                (TYPE.to_owned(), Json::Text(TEXT_TYPE.to_owned())),
                (TEXT.to_owned(), Json::Text(text)),
            ]))]),
        );
        return Ok(Json::Object(modified));
    }
    modified.insert(CONTENT.to_owned(), Json::List(kept));
    Ok(Json::Object(modified))
}

fn skip_blank_text_content(
    message: &Json,
    fields: &Object,
    text: &str,
) -> Result<Json, ContentError> {
    if !is_blank(text) || !has_non_content_values(fields) {
        return Ok(message.clone());
    }
    let Some(role) = fields.get(ROLE).and_then(Json::as_str) else {
        return Err(ContentError::MessageWithoutRole);
    };
    if role != "assistant" {
        return Ok(message.clone());
    }
    let mut modified = fields.clone();
    modified.insert(CONTENT.to_owned(), Json::Null);
    Ok(Json::Object(modified))
}

fn skip_text_block_list(fields: &Object, parts: &[Json]) -> Result<Json, ContentError> {
    let mut kept = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        if part_kept_by_empty_filter(part, index)? {
            kept.push(part.clone());
        }
    }
    let Some(role) = fields.get(ROLE).and_then(Json::as_str) else {
        return Err(ContentError::MessageWithoutRole);
    };
    if kept.is_empty() && role == "assistant" {
        let mut modified = fields.clone();
        modified.insert(CONTENT.to_owned(), Json::Null);
        return Ok(Json::Object(modified));
    }
    let mut modified = fields.clone();
    if matches!(role, "assistant" | "user") {
        modified.insert(CONTENT.to_owned(), Json::List(kept));
    }
    Ok(Json::Object(modified))
}

fn part_kept_by_empty_filter(part: &Json, index: usize) -> Result<bool, ContentError> {
    let Some(fields) = part.as_object() else {
        return Err(ContentError::PartNotObject { index });
    };
    if fields.get(TYPE).and_then(Json::as_str) != Some(TEXT_TYPE) {
        return Ok(true);
    }
    match fields.get(TEXT) {
        None => Ok(false),
        Some(Json::Text(text)) => Ok(!is_blank(text)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ContentError::TextPartNotText { index })
        }
    }
}

fn part_is_empty_text(part: &Json, index: usize) -> Result<bool, ContentError> {
    let Some(fields) = part.as_object() else {
        return Err(ContentError::PartNotObject { index });
    };
    let kind = fields
        .get(TYPE)
        .ok_or(ContentError::PartWithoutType { index })?;
    if kind.as_str() != Some(TEXT_TYPE) {
        return Ok(false);
    }
    let text_value = fields
        .get(TEXT)
        .ok_or(ContentError::TextPartWithoutText { index })?;
    let Json::Text(text) = text_value else {
        return Err(ContentError::TextPartNotText { index });
    };
    Ok(is_blank(text))
}

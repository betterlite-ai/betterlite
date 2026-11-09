use std::collections::{HashMap, HashSet};

use crate::arguments::json::{Json, Object};

use super::error::ContentError;
use super::messages::{CONTENT, ROLE, TEXT};

const CACHE_POINT: &str = "cachePoint";
const DOCUMENT: &str = "document";
const DOCUMENT_PLACEHOLDER_TEXT: &str = ".";
const NAME: &str = "name";
const USER: &str = "user";

pub fn rename_duplicate_documents(contents: &[Json]) -> Result<Vec<Json>, ContentError> {
    let mut used_names = collect_document_names(contents)?;
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut renamed = Vec::with_capacity(contents.len());
    for (index, message) in contents.iter().enumerate() {
        renamed.push(rename_message_documents(
            message,
            index,
            &mut used_names,
            &mut counts,
        )?);
    }
    Ok(renamed)
}

pub fn with_text_when_document_only(message: &Json) -> Result<Json, ContentError> {
    let Some(fields) = message.as_object() else {
        return Err(ContentError::MessageNotObject);
    };
    let content = fields.get(CONTENT);
    let Some(Json::List(blocks)) = content else {
        return Err(match content {
            None => ContentError::MessageWithoutContent,
            Some(_) => ContentError::MessageContentNotList,
        });
    };
    let Some(role) = fields.get(ROLE).and_then(Json::as_str) else {
        return Err(ContentError::MessageWithoutRole);
    };
    if role != USER {
        return Ok(message.clone());
    }
    let mut has_document = false;
    let mut any_block_has_text = false;
    for (index, block) in blocks.iter().enumerate() {
        let Some(block_fields) = block.as_object() else {
            return Err(ContentError::BlockNotObject { index });
        };
        has_document |= block_fields.contains_key(DOCUMENT);
        any_block_has_text |= block_fields.contains_key(TEXT);
    }
    if !has_document || any_block_has_text {
        return Ok(message.clone());
    }
    Ok(insert_document_placeholder(blocks))
}

fn insert_document_placeholder(blocks: &[Json]) -> Json {
    let cut = if blocks
        .last()
        .and_then(Json::as_object)
        .is_some_and(|last_fields| last_fields.contains_key(CACHE_POINT))
    {
        blocks.len().saturating_sub(1)
    } else {
        blocks.len()
    };
    let placeholder = Json::Object(Object::from_iter([(
        TEXT.to_owned(),
        Json::Text(DOCUMENT_PLACEHOLDER_TEXT.to_owned()),
    )]));
    let mut rewritten = Vec::with_capacity(blocks.len().saturating_add(1));
    for (index, block) in blocks.iter().enumerate() {
        if index == cut {
            rewritten.push(placeholder.clone());
        }
        rewritten.push(block.clone());
    }
    if cut == blocks.len() {
        rewritten.push(placeholder);
    }
    Json::Object(Object::from_iter([
        (ROLE.to_owned(), Json::Text(USER.to_owned())),
        (CONTENT.to_owned(), Json::List(rewritten)),
    ]))
}

fn collect_document_names(contents: &[Json]) -> Result<HashSet<String>, ContentError> {
    let mut names = HashSet::new();
    for (index, message) in contents.iter().enumerate() {
        for (block_index, block) in message_blocks(message, index)?.iter().enumerate() {
            let Some(fields) = block.as_object() else {
                return Err(ContentError::BlockNotObject { index: block_index });
            };
            let Some(Json::Object(document)) = fields.get(DOCUMENT) else {
                continue;
            };
            let Some(name) = document_name(document)? else {
                continue;
            };
            names.insert(name.to_owned());
        }
    }
    Ok(names)
}

fn rename_message_documents(
    message: &Json,
    index: usize,
    used_names: &mut HashSet<String>,
    counts: &mut HashMap<String, usize>,
) -> Result<Json, ContentError> {
    let blocks = message_blocks(message, index)?;
    if blocks.is_empty() {
        return Ok(message.clone());
    }
    let mut renamed_blocks = Vec::with_capacity(blocks.len());
    for (block_index, block) in blocks.iter().enumerate() {
        renamed_blocks.push(rename_block_documents(
            block,
            block_index,
            used_names,
            counts,
        )?);
    }
    let Some(fields) = message.as_object() else {
        return Err(ContentError::MessageAtNotObject { index });
    };
    let mut modified = fields.clone();
    modified.insert(CONTENT.to_owned(), Json::List(renamed_blocks));
    Ok(Json::Object(modified))
}

fn rename_block_documents(
    block: &Json,
    index: usize,
    used_names: &mut HashSet<String>,
    counts: &mut HashMap<String, usize>,
) -> Result<Json, ContentError> {
    let Some(fields) = block.as_object() else {
        return Err(ContentError::BlockNotObject { index });
    };
    let Some(Json::Object(document)) = fields.get(DOCUMENT) else {
        return Ok(block.clone());
    };
    let Some(name) = document_name(document)? else {
        return Ok(block.clone());
    };
    let count = counts.entry(name.to_owned()).or_insert(0_usize);
    let ordinal = count.checked_add(1).ok_or(ContentError::CounterOverflow)?;
    *count = ordinal;
    if ordinal < 2 {
        return Ok(block.clone());
    }
    let mut suffix = ordinal;
    let mut candidate = format!("{name}_{suffix}");
    while used_names.contains(&candidate) {
        suffix = suffix.checked_add(1).ok_or(ContentError::CounterOverflow)?;
        candidate = format!("{name}_{suffix}");
    }
    used_names.insert(candidate.clone());
    let mut renamed_document = document.clone();
    renamed_document.insert(NAME.to_owned(), Json::Text(candidate));
    let mut renamed_block = fields.clone();
    renamed_block.insert(DOCUMENT.to_owned(), Json::Object(renamed_document));
    Ok(Json::Object(renamed_block))
}

fn document_name(document: &Object) -> Result<Option<&str>, ContentError> {
    match document.get(NAME) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(name)) if name.is_empty() => Ok(None),
        Some(Json::Text(name)) => Ok(Some(name)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ContentError::DocumentNameNotText)
        }
    }
}

fn message_blocks(message: &Json, index: usize) -> Result<&[Json], ContentError> {
    let fields = message
        .as_object()
        .ok_or(ContentError::MessageAtNotObject { index })?;
    match fields.get(CONTENT) {
        None | Some(Json::Null) => Ok(&[]),
        Some(Json::List(blocks)) => Ok(blocks),
        Some(Json::Text(content)) if content.is_empty() => Ok(&[]),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(ContentError::MessageContentNotListAt { index })
        }
    }
}

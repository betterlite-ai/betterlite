use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::gemini_tool_results::types::LastToolCallMessage;
use betterlite_prompts::message_content::{Content, Message};

use crate::gemini_history::content::{bounded, decode_thinking_part, object};
use crate::gemini_history::error::HistoryError;
use crate::gemini_history::media_source::{
    append_user_part, generated_image, push_source, text_of,
};
use crate::gemini_history::tool_turn::{
    append_invocations, append_tool_calls, collect_tool_signatures,
};
use crate::gemini_media::error::MediaField;

const TEXT: &str = "text";
const REASONING_CONTENT: &str = "reasoning_content";
const IMAGES: &str = "images";
const THINKING_BLOCKS: &str = "thinking_blocks";
const PROVIDER_FIELDS: &str = "provider_specific_fields";
const THOUGHT_SIGNATURES: &str = "thought_signatures";

#[derive(Clone, Copy)]
pub struct AssistantOptions<'model> {
    pub model: Option<&'model str>,
    pub forward_function_call_id: bool,
}

pub fn append_user_parts(
    message: &Message,
    model: Option<&str>,
    parts: &mut Vec<Object>,
) -> Result<(), HistoryError> {
    match &message.content {
        None => Ok(()),
        Some(Content::Text(text)) => {
            parts.push(object!(TEXT => Json::Text(text.clone())));
            Ok(())
        }
        Some(Content::Parts(items)) => bounded(items)?
            .iter()
            .try_for_each(|item| append_user_part(item, model, parts)),
    }
}

pub fn append_assistant_parts(
    message: &Message,
    options: AssistantOptions<'_>,
    parts: &mut Vec<Object>,
    last_tool_calls: &mut Option<LastToolCallMessage>,
) -> Result<(), HistoryError> {
    let fields = &message.provider_specific_fields;
    if let Some(text) = text_of(fields, REASONING_CONTENT) {
        parts.push(object!("thought" => Json::Bool(true), TEXT => Json::Text(text)));
    }
    for block in blocks_of(fields)? {
        let Some(entry) = block.as_object() else {
            continue;
        };
        if entry.get("type").and_then(Json::as_str) == Some("thinking")
            && let (Some(thinking), Some(signature)) = (
                entry.get("thinking").and_then(Json::as_str),
                entry.get("signature").and_then(Json::as_str),
            )
        {
            parts.push(decode_thinking_part(thinking, signature));
        }
    }
    append_content(message, fields, parts)?;
    append_images(fields, options.model, parts)?;
    append_tool_calls(message, fields, options, parts, last_tool_calls)?;
    append_invocations(fields, parts)
}

fn blocks_of(fields: &Object) -> Result<&[Json], HistoryError> {
    if let Some(Json::List(blocks)) = fields.get(THINKING_BLOCKS) {
        return bounded(blocks);
    }
    Ok(&[])
}

fn append_content(
    message: &Message,
    fields: &Object,
    parts: &mut Vec<Object>,
) -> Result<(), HistoryError> {
    if let Some(Content::Text(text)) = &message.content {
        let mut part = object!(TEXT => Json::Text(text.clone()));
        if let Some(signature) = text_signature(fields)? {
            part.insert("thoughtSignature".to_owned(), signature);
        }
        parts.push(part);
    }
    if let Some(Content::Parts(items)) = &message.content {
        for item in bounded(items)? {
            if item.kind.as_str() == TEXT
                && let Some(text) = text_of(&item.fields, TEXT)
            {
                parts.push(object!(TEXT => Json::Text(text)));
            }
        }
    }
    Ok(())
}

fn text_signature(fields: &Object) -> Result<Option<Json>, HistoryError> {
    let Some(Json::Object(provider)) = fields.get(PROVIDER_FIELDS) else {
        return Ok(None);
    };
    let Some(Json::List(signatures)) = provider.get(THOUGHT_SIGNATURES) else {
        return Ok(None);
    };
    let Some(signature) = signatures.first() else {
        return Ok(None);
    };
    let tool_signatures = collect_tool_signatures(fields)?;
    let unhashable = matches!(signature, Json::List(_) | Json::Object(_));
    if unhashable || matches!(signature, Json::Text(raw) if tool_signatures.contains(raw)) {
        return Ok(None);
    }
    Ok(Some(signature.clone()))
}

pub fn append_images(
    fields: &Object,
    model: Option<&str>,
    parts: &mut Vec<Object>,
) -> Result<(), HistoryError> {
    let Some(Json::List(items)) = fields.get(IMAGES) else {
        return Ok(());
    };
    let mut locators = bounded(items)?.iter().filter_map(generated_image);
    locators.try_for_each(|locator| push_source(parts, &locator, MediaField::ImageUrl, model))
}

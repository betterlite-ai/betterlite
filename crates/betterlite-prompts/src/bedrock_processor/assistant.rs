use std::slice;

use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::{Json, Object};
use crate::bedrock_content::continuation::assistant_message_or_continue;
use crate::bedrock_content::dedup::{TOOL_USE_BLOCK, TOOL_USE_ID, deduplicate_content_blocks};
use crate::bedrock_content::order::sort_assistant_content_blocks;
use crate::bedrock_tool_results::invoke::{CachePoints, tool_call_invoke_blocks};

use super::blocks::{
    ASSISTANT, CONTENT, IMAGE_URL, Role, TEXT, TOOL_CALLS, append_cache_point, as_json, block_kind,
    is_blank, object_of, role_at, text_block,
};
use super::conversation::Conversion;
use super::error::ProcessorError;
use super::media::{image_block, image_source};
use super::thinking::{append_thinking_blocks, translate_thinking_blocks};

const THINKING: &str = "thinking";
const THINKING_BLOCKS: &str = "thinking_blocks";

fn optional_block_text(fields: &Object, block: usize) -> Result<Option<&str>, ProcessorError> {
    match fields.get(TEXT) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(ProcessorError::TextNotText { block }),
    }
}

fn part_text(fields: &Object, block: usize) -> Result<Option<&str>, ProcessorError> {
    match optional_block_text(fields, block)? {
        Some(text) if !is_blank(text) => Ok(Some(text)),
        Some(_) | None => Ok(None),
    }
}

fn tool_invokes<R, C>(
    message_fields: &Object,
    message_index: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<Vec<Json>, ProcessorError>
where
    C: CachePoints,
{
    let Some(calls) = message_fields.get(TOOL_CALLS) else {
        return Ok(Vec::new());
    };
    if matches!(calls, Json::Null) {
        return Ok(Vec::new());
    }
    let Json::List(tool_calls) = calls else {
        return Err(ProcessorError::ToolCallsNotList {
            message: message_index,
        });
    };
    if tool_calls.is_empty() {
        return Ok(Vec::new());
    }
    let mut invokes = Vec::new();
    for invoke in tool_call_invoke_blocks(tool_calls, conversion.model, conversion.cache_points)? {
        invokes.push(as_json(&invoke)?);
    }
    Ok(invokes)
}

async fn assistant_content_blocks<R, C>(
    content: &mut Vec<Json>,
    fields: &Object,
    index: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<(), ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
{
    match fields.get(CONTENT) {
        Some(Json::List(parts)) => {
            for (block, part) in parts.iter().enumerate() {
                let part_fields =
                    object_of(part).ok_or(ProcessorError::BlockNotObject { block })?;
                match block_kind(part_fields, block)? {
                    THINKING => {
                        let translated = translate_thinking_blocks(slice::from_ref(part))?;
                        append_thinking_blocks(&translated, content)?;
                    }
                    TEXT => content.extend(part_text(part_fields, block)?.map(text_block)),
                    IMAGE_URL => {
                        let (image_url, _format) = image_source(part_fields, block)?;
                        content.push(image_block(image_url, None, block, conversion).await?);
                    }
                    _ => {}
                }
                append_cache_point(content, part_fields, conversion);
            }
        }
        Some(Json::Text(text)) => {
            if !is_blank(text) {
                content.push(text_block(text));
            }
            append_cache_point(content, fields, conversion);
        }
        Some(Json::Null) | None => {}
        Some(_) => return Err(ProcessorError::ContentNotTextOrList { message: index }),
    }
    content.extend(tool_invokes(fields, index, conversion)?);
    Ok(())
}

pub(crate) async fn assistant_turn_blocks<R, C>(
    messages: &[Json],
    start: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<(Vec<Json>, usize), ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
{
    let mut index = start;
    let mut blocks = Vec::new();
    while let Some(message) = messages.get(index) {
        if !matches!(role_at(message, index)?, Role::Text(ASSISTANT)) {
            break;
        }
        let block = assistant_message_or_continue(
            message,
            conversion.assistant_continue_message,
            conversion.modify_params,
        )?;
        let fields =
            object_of(&block).ok_or(ProcessorError::MessageNotObject { message: index })?;
        match fields.get(THINKING_BLOCKS) {
            None | Some(Json::Null) => {}
            Some(Json::List(thinking_blocks)) => {
                let translated = translate_thinking_blocks(thinking_blocks)?;
                append_thinking_blocks(&translated, &mut blocks)?;
            }
            Some(_) => return Err(ProcessorError::ThinkingBlocksNotList { message: index }),
        }
        assistant_content_blocks(&mut blocks, fields, index, conversion).await?;
        index = index.saturating_add(1);
    }
    let deduplicated = deduplicate_content_blocks(&blocks, TOOL_USE_BLOCK, TOOL_USE_ID)?;
    let sorted = sort_assistant_content_blocks(&deduplicated)?;
    Ok((sorted, index))
}

use std::slice;

use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::{Json, Object};
use crate::bedrock_content::continuation::assistant_message_or_continue;
use crate::bedrock_content::dedup::{TOOL_USE_BLOCK, TOOL_USE_ID, deduplicate_content_blocks};
use crate::bedrock_content::order::sort_assistant_content_blocks;
use crate::bedrock_tool_results::invoke::{CachePoints, tool_call_invoke_blocks};

use super::blocks::{
    ASSISTANT, CONTENT, IMAGE_URL, Role, TEXT, TOOL_CALLS, append_cache_point, as_json, block_kind,
    block_object, is_blank, message_object, role_at,
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

async fn assistant_part<R, C>(
    part: &Json,
    kind: &str,
    block: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<Option<Json>, ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
{
    let fields = block_object(part, block)?;
    match kind {
        THINKING => {
            let translated = translate_thinking_blocks(slice::from_ref(part))?;
            let mut parts = Vec::new();
            append_thinking_blocks(&translated, &mut parts)?;
            Ok(parts.into_iter().next())
        }
        TEXT => match optional_block_text(fields, block)? {
            Some(text) if !is_blank(text) => Ok(Some(Json::Object(Object::from_iter([(
                TEXT.to_owned(),
                Json::Text(text.to_owned()),
            )])))),
            Some(_) | None => Ok(None),
        },
        IMAGE_URL => {
            let (image_url, _format) = image_source(fields, block)?;
            Ok(Some(image_block(image_url, None, block, conversion).await?))
        }
        _ => Ok(None),
    }
}

fn append_tool_invokes<R, C>(
    content: &mut Vec<Json>,
    fields: &Object,
    index: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<(), ProcessorError>
where
    C: CachePoints,
{
    match fields.get(TOOL_CALLS) {
        None | Some(Json::Null) => Ok(()),
        Some(Json::List(tool_calls)) if tool_calls.is_empty() => Ok(()),
        Some(Json::List(tool_calls)) => {
            for invoke in
                tool_call_invoke_blocks(tool_calls, conversion.model, conversion.cache_points)?
            {
                content.push(as_json(&invoke)?);
            }
            Ok(())
        }
        Some(_) => Err(ProcessorError::ToolCallsNotList { message: index }),
    }
}

async fn assistant_content_blocks<R, C>(
    fields: &Object,
    index: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<Vec<Json>, ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
{
    let mut content = Vec::new();
    match fields.get(THINKING_BLOCKS) {
        None | Some(Json::Null) => {}
        Some(Json::List(thinking_blocks)) => {
            let translated = translate_thinking_blocks(thinking_blocks)?;
            append_thinking_blocks(&translated, &mut content)?;
        }
        Some(_) => return Err(ProcessorError::ThinkingBlocksNotList { message: index }),
    }
    match fields.get(CONTENT) {
        Some(Json::List(parts)) => {
            for (part_index, part) in parts.iter().enumerate() {
                let part_fields = block_object(part, part_index)?;
                let kind = block_kind(part_fields, part_index)?;
                if let Some(element) = assistant_part(part, kind, part_index, conversion).await? {
                    content.push(element);
                }
                append_cache_point(&mut content, part_fields, conversion);
            }
        }
        Some(Json::Text(text)) => {
            if !is_blank(text) {
                content.push(Json::Object(Object::from_iter([(
                    TEXT.to_owned(),
                    Json::Text(text.clone()),
                )])));
            }
            append_cache_point(&mut content, fields, conversion);
        }
        Some(Json::Null) | None => {}
        Some(_) => return Err(ProcessorError::ContentNotTextOrList { message: index }),
    }
    append_tool_invokes(&mut content, fields, index, conversion)?;
    Ok(content)
}

async fn assistant_message_blocks<R, C>(
    message: &Json,
    index: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<Vec<Json>, ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
{
    let block = assistant_message_or_continue(
        message,
        conversion.assistant_continue_message,
        conversion.modify_params,
    )?;
    let fields = message_object(&block, index)?;
    assistant_content_blocks(fields, index, conversion).await
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
        blocks.extend(assistant_message_blocks(message, index, conversion).await?);
        index = index.saturating_add(1);
    }
    let deduplicated = deduplicate_content_blocks(&blocks, TOOL_USE_BLOCK, TOOL_USE_ID)?;
    let sorted = sort_assistant_content_blocks(&deduplicated)?;
    Ok((sorted, index))
}

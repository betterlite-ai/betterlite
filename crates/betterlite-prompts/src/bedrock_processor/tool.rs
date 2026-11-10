use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::Json;
use crate::bedrock_content::dedup::{TOOL_RESULT_BLOCK, TOOL_USE_ID, deduplicate_content_blocks};
use crate::bedrock_images::remote::RemoteImage;
use crate::bedrock_tool_results::invoke::CachePoints;
use crate::bedrock_tool_results::result::to_bedrock_tool_result;
use crate::message_content::Message;

use super::blocks::{
    CACHE_CONTROL, CONTENT, Role, TOOL, as_json, object_of, push_cache_point, role_at,
};
use super::conversation::Conversion;
use super::error::ProcessorError;

fn cache_control_of_part(part: &Json) -> Option<&Json> {
    let fields = part.as_object()?;
    match fields.get(CACHE_CONTROL) {
        None | Some(Json::Null) => None,
        Some(control) => Some(control),
    }
}

fn tool_cache_control(message: &Json, index: usize) -> Result<Option<&Json>, ProcessorError> {
    let fields = object_of(message).ok_or(ProcessorError::MessageNotObject { message: index })?;
    match fields.get(CACHE_CONTROL) {
        Some(Json::Null) | None => match fields.get(CONTENT) {
            Some(Json::List(parts)) => Ok(parts.iter().find_map(cache_control_of_part)),
            Some(
                Json::Null | Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::Object(_),
            )
            | None => Ok(None),
        },
        Some(control) => Ok(Some(control)),
    }
}

async fn tool_result_block<R, Ids>(
    message: &Json,
    index: usize,
    remote: &RemoteImage<R>,
    new_id: &Ids,
) -> Result<Json, ProcessorError>
where
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let value =
        serde_json::to_value(message).map_err(|source| ProcessorError::ToolMessageShape {
            message: index,
            source,
        })?;
    let typed: Message =
        serde_json::from_value(value).map_err(|source| ProcessorError::ToolMessageShape {
            message: index,
            source,
        })?;
    let result = to_bedrock_tool_result(&typed, remote, new_id)
        .await
        .map_err(|source| ProcessorError::ToolResult { source })?;
    as_json(&result)
}

async fn tool_message_blocks<R, C, Ids>(
    message: &Json,
    index: usize,
    conversion: &Conversion<'_, C, R>,
    new_id: &Ids,
) -> Result<Vec<Json>, ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
    Ids: Fn() -> String + Sync,
{
    let mut blocks = vec![tool_result_block(message, index, conversion.remote, new_id).await?];
    if let Some(control) = tool_cache_control(message, index)? {
        push_cache_point(
            &mut blocks,
            control,
            conversion.cache_points,
            conversion.model,
        );
    }
    Ok(blocks)
}

pub(crate) async fn tool_turn_blocks<R, C, Ids>(
    messages: &[Json],
    start: usize,
    conversion: &Conversion<'_, C, R>,
    new_id: &Ids,
) -> Result<(Vec<Json>, usize), ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
    Ids: Fn() -> String + Sync,
{
    let mut index = start;
    let mut blocks = Vec::new();
    while let Some(message) = messages.get(index) {
        if !matches!(role_at(message, index)?, Role::Text(TOOL)) {
            break;
        }
        blocks.extend(tool_message_blocks(message, index, conversion, new_id).await?);
        index = index.saturating_add(1);
    }
    let deduplicated = deduplicate_content_blocks(&blocks, TOOL_RESULT_BLOCK, TOOL_USE_ID)?;
    Ok((deduplicated, index))
}

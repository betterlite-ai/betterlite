use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::Json;
use crate::bedrock_content::continuation::user_message_block_or_continue;
use crate::bedrock_tool_results::invoke::CachePoints;

use super::blocks::{CONTENT, Role, USER, append_cache_point, object_of, role_at, text_block};
use super::conversation::Conversion;
use super::error::ProcessorError;
use super::user_blocks::user_list_blocks;

pub(crate) async fn user_turn_blocks<R, C>(
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
        if !matches!(role_at(message, index)?, Role::Text(USER)) {
            break;
        }
        let message_block = user_message_block_or_continue(
            message,
            conversion.user_continue_message,
            conversion.modify_params,
        )?;
        let message_fields =
            object_of(&message_block).ok_or(ProcessorError::MessageNotObject { message: index })?;
        match message_fields.get(CONTENT) {
            Some(Json::List(parts)) => user_list_blocks(&mut blocks, parts, conversion).await?,
            Some(Json::Text(text)) if !text.is_empty() => {
                blocks.push(text_block(text));
                append_cache_point(&mut blocks, message_fields, conversion);
            }
            Some(Json::Null | Json::Text(_)) | None => {}
            Some(_) => return Err(ProcessorError::ContentNotTextOrList { message: index }),
        }
        index = index.saturating_add(1);
    }
    Ok((blocks, index))
}

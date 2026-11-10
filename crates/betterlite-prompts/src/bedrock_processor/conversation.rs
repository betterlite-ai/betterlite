use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::Json;
use crate::bedrock_content::continuation::insert_assistant_continue;
use crate::bedrock_content::documents::{rename_duplicate_documents, with_text_when_document_only};
use crate::bedrock_images::remote::RemoteImage;
use crate::bedrock_tool_results::invoke::CachePoints;

use super::assistant::assistant_turn_blocks;
use super::blocks::{ASSISTANT, CONTENT, Role, USER, continue_present, message, role_at};
use super::error::ProcessorError;
use super::setup::initial_message_setup;
use super::tool::tool_turn_blocks;
use super::user::user_turn_blocks;

pub struct Conversion<'call, C, R> {
    pub model: Option<&'call str>,
    pub cache_points: &'call C,
    pub remote: &'call RemoteImage<R>,
    pub user_continue_message: Option<&'call Json>,
    pub assistant_continue_message: Option<&'call Json>,
    pub modify_params: bool,
}

fn finalize(contents: &[Json]) -> Result<Vec<Json>, ProcessorError> {
    let renamed = rename_duplicate_documents(contents)?;
    let mut ensured = Vec::with_capacity(renamed.len());
    for renamed_message in &renamed {
        ensured.push(with_text_when_document_only(renamed_message)?);
    }
    Ok(ensured)
}

pub(crate) fn push_user_blocks(
    contents: &mut Vec<Json>,
    blocks: Vec<Json>,
    assistant_continue_message: Option<&Json>,
    modify_params: bool,
) -> Result<(), ProcessorError> {
    if blocks.is_empty() {
        return Ok(());
    }
    let last_index = contents.len().saturating_sub(1);
    let last_is_user = match contents.last() {
        Some(last) => matches!(role_at(last, last_index)?, Role::Text(USER)),
        None => false,
    };
    if !last_is_user {
        contents.push(message(USER, blocks));
        return Ok(());
    }
    if continue_present(assistant_continue_message) || modify_params {
        *contents = insert_assistant_continue(contents, assistant_continue_message, modify_params);
        contents.push(message(USER, blocks));
        return Ok(());
    }
    if let Some(Json::Object(fields)) = contents.last_mut()
        && let Some(Json::List(existing)) = fields.get_mut(CONTENT)
    {
        tracing::warn!(
            "consecutive user or tool blocks are merged; set assistant_continue_message or modify_params to insert a continue message"
        );
        existing.extend(blocks);
        return Ok(());
    }
    Err(ProcessorError::MergedContentNotList {
        message: last_index,
    })
}

async fn convert_round<R, C, Ids>(
    prepared: &[Json],
    start: usize,
    conversion: &Conversion<'_, C, R>,
    new_id: &Ids,
    contents: &mut Vec<Json>,
) -> Result<usize, ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
    Ids: Fn() -> String + Sync,
{
    let (user_blocks, after_user) = user_turn_blocks(prepared, start, conversion).await?;
    push_user_blocks(
        contents,
        user_blocks,
        conversion.assistant_continue_message,
        conversion.modify_params,
    )?;
    let (tool_blocks, after_tool) =
        tool_turn_blocks(prepared, after_user, conversion, new_id).await?;
    push_user_blocks(
        contents,
        tool_blocks,
        conversion.assistant_continue_message,
        conversion.modify_params,
    )?;
    let (assistant_blocks, after_assistant) =
        assistant_turn_blocks(prepared, after_tool, conversion).await?;
    if !assistant_blocks.is_empty() {
        contents.push(message(ASSISTANT, assistant_blocks));
    }
    Ok(after_assistant)
}

pub async fn converse_messages<R, C, Ids>(
    messages: &[Json],
    conversion: &Conversion<'_, C, R>,
    new_id: &Ids,
) -> Result<Vec<Json>, ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
    Ids: Fn() -> String + Sync,
{
    let prepared = initial_message_setup(
        messages,
        conversion.user_continue_message,
        conversion.modify_params,
    )?;
    let mut contents: Vec<Json> = Vec::new();
    let mut index = 0_usize;
    while let Some(current) = prepared.get(index) {
        let start = index;
        index = convert_round(&prepared, start, conversion, new_id, &mut contents).await?;
        if index == start {
            let role = match role_at(current, start)? {
                Role::Text(role) => role.to_owned(),
                Role::Null => "null".to_owned(),
            };
            return Err(ProcessorError::UnsupportedRole {
                message: start,
                role,
            });
        }
    }
    finalize(&contents)
}

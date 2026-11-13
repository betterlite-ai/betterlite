use crate::anthropic_history::thinking::drop_unsignable;
use crate::anthropic_invokes::invoke::MAX_TOOL_CALLS;
use crate::arguments::json::{Json, Object};

use super::{
    AssistantError, CONTENT, ContentField, ID, MAX_PROVIDER_BLOCKS, REDACTED_THINKING,
    SERVER_TOOL_PREFIX, THINKING, THINKING_BLOCKS, TOOL_CALLS, TYPE,
};

pub(super) fn read_thinking(
    fields: &Object,
    index: usize,
) -> Result<Option<Vec<Json>>, AssistantError> {
    let Some(blocks) = list_value(fields, THINKING_BLOCKS, index)? else {
        return Ok(None);
    };
    check_bound(blocks.len(), index, THINKING_BLOCKS, MAX_PROVIDER_BLOCKS)?;
    ensure_objects(blocks, index)?;
    Ok(Some(drop_unsignable(blocks)))
}

pub(super) fn read_calls(fields: &Object, index: usize) -> Result<Option<&[Json]>, AssistantError> {
    let Some(calls) = list_value(fields, TOOL_CALLS, index)? else {
        return Ok(None);
    };
    check_bound(calls.len(), index, TOOL_CALLS, MAX_TOOL_CALLS)?;
    Ok(Some(calls))
}

pub(super) fn read_content(
    fields: &Object,
    index: usize,
) -> Result<ContentField<'_>, AssistantError> {
    match fields.get(CONTENT) {
        Some(Json::Text(text)) => Ok(ContentField::Text(text)),
        Some(Json::List(blocks)) => Ok(ContentField::Blocks(blocks)),
        Some(Json::Null) | None => Ok(ContentField::Missing),
        Some(_) => Err(AssistantError::Field {
            index,
            field: CONTENT,
        }),
    }
}

fn list_value<'input>(
    fields: &'input Object,
    field: &'static str,
    index: usize,
) -> Result<Option<&'input [Json]>, AssistantError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(items)) => Ok(Some(items)),
        Some(_) => Err(AssistantError::Field { index, field }),
    }
}

const fn check_bound(
    length: usize,
    index: usize,
    field: &'static str,
    limit: usize,
) -> Result<(), AssistantError> {
    if length > limit {
        return Err(AssistantError::TooMany {
            index,
            field,
            limit,
        });
    }
    Ok(())
}

fn ensure_objects(blocks: &[Json], index: usize) -> Result<(), AssistantError> {
    let Some(position) = blocks
        .iter()
        .position(|block| !matches!(block, Json::Object(_)))
    else {
        return Ok(());
    };
    Err(AssistantError::BlockNotObject {
        index,
        block: position,
    })
}

pub(super) fn list_has_thinking(content: ContentField<'_>) -> bool {
    let ContentField::Blocks(blocks) = content else {
        return false;
    };
    blocks.iter().any(|block| {
        block
            .as_object()
            .and_then(|fields| fields.get(TYPE))
            .and_then(Json::as_str)
            .is_some_and(|kind| kind == THINKING || kind == REDACTED_THINKING)
    })
}

pub(crate) fn has_server_tool_call(calls: Option<&[Json]>) -> bool {
    calls.is_some_and(|declared| {
        declared.iter().any(|call| {
            call.as_object()
                .and_then(|fields| fields.get(ID))
                .and_then(Json::as_str)
                .is_some_and(|id| id.starts_with(SERVER_TOOL_PREFIX))
        })
    })
}

pub(super) fn server_results<'input>(
    provider: Option<&'input Object>,
    field: &'static str,
    index: usize,
) -> Result<&'input [Json], AssistantError> {
    let Some(holder) = provider else {
        return Ok(&[]);
    };
    Ok(list_value(holder, field, index)?.unwrap_or_default())
}

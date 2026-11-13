use crate::arguments::json::Json;
use crate::file_ids::responses::{ResponsesContent, ResponsesInput};
use crate::file_ids::tools::ResponsesTool;
use crate::message_content::{Content, Message};
use thiserror::Error;

pub const MAX_MESSAGES: usize = 1024;
pub const MAX_MESSAGE_PARTS: usize = 256;
pub const MAX_INPUT_ITEMS: usize = 1024;
pub const MAX_INPUT_CONTENT_PARTS: usize = 256;
pub const MAX_TOOLS: usize = 256;
pub const MAX_VECTOR_STORE_IDS: usize = 256;
pub const MAX_CONTAINER_FILE_IDS: usize = 256;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FileIdsError {
    #[error("messages exceeds the maximum of {limit}")]
    TooManyMessages { limit: usize },
    #[error("a message content list exceeds the maximum of {limit} parts")]
    TooManyMessageParts { limit: usize },
    #[error("responses input exceeds the maximum of {limit} items")]
    TooManyInputItems { limit: usize },
    #[error("a responses content list exceeds the maximum of {limit} parts")]
    TooManyInputParts { limit: usize },
    #[error("tools exceeds the maximum of {limit}")]
    TooManyTools { limit: usize },
    #[error("vector_store_ids exceeds the maximum of {limit}")]
    TooManyVectorStoreIds { limit: usize },
    #[error("container file_ids exceeds the maximum of {limit}")]
    TooManyContainerFileIds { limit: usize },
}

const fn check_count(
    count: usize,
    limit: usize,
    too_long: FileIdsError,
) -> Result<(), FileIdsError> {
    if count > limit {
        return Err(too_long);
    }
    Ok(())
}

pub fn check_messages(messages: &[Message]) -> Result<(), FileIdsError> {
    check_count(
        messages.len(),
        MAX_MESSAGES,
        FileIdsError::TooManyMessages {
            limit: MAX_MESSAGES,
        },
    )?;
    let part_lists = messages
        .iter()
        .filter_map(|message| match &message.content {
            Some(Content::Parts(parts)) => Some(parts),
            _ => None,
        });
    for parts in part_lists {
        check_count(
            parts.len(),
            MAX_MESSAGE_PARTS,
            FileIdsError::TooManyMessageParts {
                limit: MAX_MESSAGE_PARTS,
            },
        )?;
    }
    Ok(())
}

pub fn check_responses_input(input: &ResponsesInput) -> Result<(), FileIdsError> {
    let ResponsesInput::Items(items) = input else {
        return Ok(());
    };
    check_count(
        items.len(),
        MAX_INPUT_ITEMS,
        FileIdsError::TooManyInputItems {
            limit: MAX_INPUT_ITEMS,
        },
    )?;
    for item in items {
        let Some(ResponsesContent::Parts(parts)) = &item.content else {
            continue;
        };
        check_count(
            parts.len(),
            MAX_INPUT_CONTENT_PARTS,
            FileIdsError::TooManyInputParts {
                limit: MAX_INPUT_CONTENT_PARTS,
            },
        )?;
    }
    Ok(())
}

pub fn check_tools(tools: Option<&[ResponsesTool]>) -> Result<(), FileIdsError> {
    let Some(present) = tools else {
        return Ok(());
    };
    check_count(
        present.len(),
        MAX_TOOLS,
        FileIdsError::TooManyTools { limit: MAX_TOOLS },
    )?;
    for tool in present {
        if tool.kind.as_deref() == Some("file_search")
            && let Some(Json::List(ids)) = tool.fields.get("vector_store_ids")
        {
            check_count(
                ids.len(),
                MAX_VECTOR_STORE_IDS,
                FileIdsError::TooManyVectorStoreIds {
                    limit: MAX_VECTOR_STORE_IDS,
                },
            )?;
        }
        if tool.kind.as_deref() == Some("code_interpreter")
            && let Some(Json::Object(container)) = tool.fields.get("container")
            && let Some(Json::List(file_ids)) = container.get("file_ids")
        {
            check_count(
                file_ids.len(),
                MAX_CONTAINER_FILE_IDS,
                FileIdsError::TooManyContainerFileIds {
                    limit: MAX_CONTAINER_FILE_IDS,
                },
            )?;
        }
    }
    Ok(())
}

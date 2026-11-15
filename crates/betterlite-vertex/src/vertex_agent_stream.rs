use betterlite_models::decisions::{Extra, Json};
use betterlite_streaming::stream_chunk_shaping::tokens::mapped_finish_reason;
use thiserror::Error;

// LiteLLM names the assistant on a chunk whose text is non-empty, and leaves the role out otherwise
pub const ASSISTANT_ROLE: &str = "assistant";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentEngineUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

// one Agent Engine stream chunk, projected the way LiteLLM's chunk_parser and its chunk types read it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentEngineChunk<'chunk> {
    pub content: Option<&'chunk str>,
    pub role: Option<&'static str>,
    pub finish_reason: Option<&'static str>,
    pub usage: Option<AgentEngineUsage>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgentEngineStreamError {
    #[error("the Vertex Agent Engine chunk field {field} has the wrong type")]
    FieldType { field: &'static str },
    #[error("the Vertex Agent Engine chunk field {field} does not carry a token count")]
    UsageCount { field: &'static str },
}

pub fn parse_chunk(fields: &Extra) -> Result<AgentEngineChunk<'_>, AgentEngineStreamError> {
    let content = content_text(fields)?;
    Ok(AgentEngineChunk {
        content,
        role: content
            .is_some_and(|text| !text.is_empty())
            .then_some(ASSISTANT_ROLE),
        finish_reason: finish_reason(fields)?,
        usage: usage(fields)?,
    })
}

// the first part that is an object carrying a text key decides, and no later part is read
fn content_text(fields: &Extra) -> Result<Option<&str>, AgentEngineStreamError> {
    let Some(content_value) = fields.get("content") else {
        return Ok(None);
    };
    let content_object = match content_value {
        Json::Null => return Ok(None),
        Json::Object(content_fields) => content_fields,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(AgentEngineStreamError::FieldType { field: "content" });
        }
    };
    let Some(parts_value) = content_object.get("parts") else {
        return Ok(None);
    };
    let part_list = match parts_value {
        Json::Null => return Ok(None),
        Json::List(part_items) => part_items,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            return Err(AgentEngineStreamError::FieldType { field: "parts" });
        }
    };
    for part in part_list {
        if let Json::Object(part_fields) = part
            && let Some(text_value) = part_fields.get("text")
        {
            return match text_value {
                Json::Text(text_content) => Ok(Some(text_content)),
                Json::Null => Ok(None),
                Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(AgentEngineStreamError::FieldType { field: "text" })
                }
            };
        }
    }
    Ok(None)
}

// LiteLLM lowercases the provider's reason and its chunk types then map that reason onto OpenAI's set
fn finish_reason(fields: &Extra) -> Result<Option<&'static str>, AgentEngineStreamError> {
    match fields.get("finish_reason") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(reason)) if reason.is_empty() => Ok(None),
        Some(Json::Text(reason)) => Ok(Some(mapped_finish_reason(&reason.to_lowercase()))),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(AgentEngineStreamError::FieldType {
                field: "finish_reason",
            })
        }
    }
}

// an empty usage_metadata is falsy in LiteLLM and leaves the chunk without usage
fn usage(fields: &Extra) -> Result<Option<AgentEngineUsage>, AgentEngineStreamError> {
    let metadata = match fields.get("usage_metadata") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Object(metadata)) => metadata,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(AgentEngineStreamError::FieldType {
                field: "usage_metadata",
            });
        }
    };
    if metadata.is_empty() {
        return Ok(None);
    }
    Ok(Some(AgentEngineUsage {
        prompt_tokens: count(metadata, "prompt_token_count")?,
        completion_tokens: count(metadata, "candidates_token_count")?,
        total_tokens: count(metadata, "total_token_count")?,
    }))
}

// a count the metadata left out is zero; keys LiteLLM's usage block does not read stay unread
fn count(metadata: &Extra, field: &'static str) -> Result<u64, AgentEngineStreamError> {
    let Some(value) = metadata
        .get(field)
        .filter(|value| !matches!(value, Json::Null))
    else {
        return Ok(0);
    };
    let Json::Number(number) = value else {
        return Err(AgentEngineStreamError::FieldType { field });
    };
    number
        .as_u64()
        .ok_or(AgentEngineStreamError::UsageCount { field })
}

mod tests;

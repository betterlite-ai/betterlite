use serde::Serialize;

use super::error::BlockFramingError;

#[derive(Debug, Default, Serialize)]
pub struct BlockedChunkDelta<'chunk> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<&'chunk str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<&'chunk str>,
}

#[derive(Debug, Serialize)]
pub struct BlockedChunkChoice<'chunk> {
    pub index: i64,
    pub delta: BlockedChunkDelta<'chunk>,
    pub finish_reason: Option<&'chunk str>,
}

#[derive(Debug, Serialize)]
pub struct BlockedChunkUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct BlockedChunk<'chunk> {
    pub id: &'chunk str,
    pub object: &'chunk str,
    pub created: i64,
    pub model: &'chunk str,
    pub choices: [BlockedChunkChoice<'chunk>; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<BlockedChunkUsage>,
}

pub fn chat_sse_chunk(payload: &BlockedChunk<'_>) -> Result<Vec<u8>, BlockFramingError> {
    let body =
        serde_json::to_string(payload).map_err(|source| BlockFramingError::Serialize { source })?;
    Ok(format!("data: {body}\n\n").into_bytes())
}

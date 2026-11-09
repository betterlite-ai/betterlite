use std::str;

use betterlite_models::decisions::Json;

use crate::agentcore_response::content::AgentCoreMessage;
use crate::agentcore_response::error::AgentCoreReplyError;
use crate::agentcore_response::json::parse_json_reply;
use crate::agentcore_response::sse::parse_sse_stream;
use crate::agentcore_response::usage::AgentCoreUsage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReply {
    pub content: String,
    pub usage: Option<AgentCoreUsage>,
    pub final_message: Option<AgentCoreMessage>,
}

pub fn decode_agentcore_reply(
    body: &[u8],
    content_type: Option<&str>,
) -> Result<ParsedReply, AgentCoreReplyError> {
    let text = str::from_utf8(body).map_err(|source| AgentCoreReplyError::NotUtf8 { source })?;
    if content_type.is_some_and(|value| value.to_lowercase().contains("application/json")) {
        let value: Json =
            serde_json::from_str(text).map_err(|source| AgentCoreReplyError::Json { source })?;
        parse_json_reply(&value)
    } else {
        parse_sse_stream(text)
    }
}

use betterlite_bedrock::converse_events::wire::non_empty;
use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatUsage;
use betterlite_openai::stream::chunk::{Choice, Chunk, Delta};
use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use betterlite_vertex::vertex_agent_stream::{AgentEngineChunk, AgentEngineUsage, parse_chunk};

use super::{IdSource, VertexAgentStreamError};

const CHUNK_OBJECT: &str = "chat.completion.chunk";
const DONE_MARKER: &str = "[DONE]";
const FINISH_STOP: &str = "stop";

pub(super) fn parse_line(
    line: &str,
    model: &str,
    ids: &dyn IdSource,
) -> Result<Option<Chunk>, VertexAgentStreamError> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    if line.contains(DONE_MARKER) {
        return Ok(Some(done_chunk(model, ids)));
    }
    let payload = strip_sse_data_from_chunk(Some(line)).unwrap_or(line);
    let Ok(parsed) = serde_json::from_str::<Json>(payload) else {
        tracing::debug!("Skipping a Vertex Agent Engine stream line that is not JSON");
        return Ok(None);
    };
    if !non_empty(&parsed) {
        return Ok(None);
    }
    let Json::Object(fields) = parsed else {
        return Err(VertexAgentStreamError::ChunkNotObject);
    };
    Ok(Some(chunk_of(model, ids, &parse_chunk(&fields)?)))
}

// the generic iterator answers the done marker with a finished chunk and no text
fn done_chunk(model: &str, ids: &dyn IdSource) -> Chunk {
    chunk_of(
        model,
        ids,
        &AgentEngineChunk {
            content: None,
            role: None,
            finish_reason: Some(FINISH_STOP),
            usage: None,
        },
    )
}

fn chunk_of(model: &str, ids: &dyn IdSource, chunk: &AgentEngineChunk<'_>) -> Chunk {
    Chunk {
        id: format!("{CHAT_COMPLETION_ID_PREFIX}{}", ids.next_id()),
        object: CHUNK_OBJECT.to_owned(),
        created: 0,
        model: model.to_owned(),
        choices: vec![Choice {
            index: 0,
            delta: Delta {
                role: chunk.role.map(str::to_owned),
                content: chunk.content.map(str::to_owned),
                refusal: None,
                tool_calls: None,
                function_call: None,
            },
            finish_reason: chunk.finish_reason.map(str::to_owned),
            logprobs: None,
        }],
        usage: chunk.usage.map(chat_usage),
        system_fingerprint: None,
        service_tier: None,
    }
}

fn chat_usage(usage: AgentEngineUsage) -> ChatUsage {
    ChatUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    }
}

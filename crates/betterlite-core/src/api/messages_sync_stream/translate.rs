use betterlite_prompts::arguments::json::Object;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

use crate::api::messages_chat_output::records::{ChoiceRecord, TerminalRecord, UsageRecord};
use crate::api::messages_chat_output::stream::terminal_frame;
use crate::api::messages_context_clear::result::AppliedEdit;
use crate::api::messages_stream_usage::error::StreamStateError;
use crate::api::messages_stream_usage::state::StreamState;

// B966 reads its records from the wire, so a decoded chunk reaches it through the same JSON shape.
pub(super) fn chunk_records(
    chunk: &StreamChunk,
) -> Result<(Vec<ChoiceRecord>, Option<UsageRecord>), StreamStateError> {
    let choices = serde_json::to_string(&chunk.choices)?;
    let Some(usage) = &chunk.usage else {
        return Ok((serde_json::from_str(&choices)?, None));
    };
    let usage = serde_json::to_string(usage)?;
    Ok((
        serde_json::from_str(&choices)?,
        Some(serde_json::from_str(&usage)?),
    ))
}

// LiteLLM reads _hidden_params["usage"] when the response itself carries no usage.
fn hidden_usage(chunk: &StreamChunk) -> Result<Option<UsageRecord>, StreamStateError> {
    let Some(usage) = chunk.hidden_params.get("usage") else {
        return Ok(None);
    };
    let text = serde_json::to_string(usage)?;
    Ok(Some(serde_json::from_str(&text)?))
}

// One frame per chunk: a finish reason closes the message and anything else is a content-block delta.
pub(super) fn event<S>(
    state: &StreamState<S>,
    chunk: &StreamChunk,
    choices: &[ChoiceRecord],
    usage: Option<&UsageRecord>,
    edits: Vec<AppliedEdit>,
) -> Result<Object, StreamStateError> {
    let record = TerminalRecord {
        choices: choices.to_vec(),
        usage: usage.cloned(),
        hidden_usage: hidden_usage(chunk)?,
        index: state.current_content_block_index,
        applied_edits: edits,
    };
    let frame = terminal_frame(&record)?;
    let text = serde_json::to_string(&frame)?;
    Ok(serde_json::from_str(&text)?)
}

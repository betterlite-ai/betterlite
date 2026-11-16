use betterlite_core::api::messages_stream_split::ChunkSource as _;
use betterlite_core::api::messages_stream_usage::state::StreamState;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use serde_json::{Value, json};
use uuid::Uuid;

use super::source::ScriptedSource;

const NORMALIZED_ID: &str = "msg_<uuid>";

// The wrapper mints a fresh uuid per stream, so the two sides compare ids only by their shape.
fn normalized_id(value: &str) -> String {
    let Some(rest) = value.strip_prefix("msg_") else {
        return value.to_owned();
    };
    if Uuid::parse_str(rest).is_err() {
        return value.to_owned();
    }
    NORMALIZED_ID.to_owned()
}

pub(super) fn snapshot(state: &StreamState<ScriptedSource>, logged_id: Option<&str>) -> Value {
    json!({
        "message_id": normalized_id(&state.message_id),
        "logged_message_id": logged_id.map(normalized_id),
        "logged_is_message_id": logged_id == Some(state.message_id.as_str()),
        "model": state.model,
        "index": state.current_content_block_index,
        "type": state.current_content_block_type,
        "start": state.current_content_block_start,
        "sent_first_chunk": state.message_progress.sent_first_chunk,
        "sent_last_message": state.message_progress.sent_last_message,
        "sent_content_block_start": state.block_progress.sent_content_block_start,
        "sent_content_block_finish": state.block_progress.sent_content_block_finish,
        "queued_usage_chunk": state.queued_usage_chunk,
        "sent_compaction_block": state.sent_compaction_block,
        "sent_compaction_block_start": state.compaction_progress.sent_compaction_block_start,
        "sent_compaction_block_delta": state.compaction_progress.sent_compaction_block_delta,
        "refusal_text": state.refusal_text,
        "tool_name_mapping": state.tool_name_mapping,
        "applied_edits": state.applied_edits,
        "compaction_block": state.compaction_block,
        "iterations_usage": state.iterations_usage,
        "held_content": state.holding_chunk,
        "held_stop": state.holding_stop_reason_chunk,
        "chunks_len": state.completion_stream.chunks().map(<[StreamChunk]>::len),
        "messages": state.completion_stream.messages(),
    })
}

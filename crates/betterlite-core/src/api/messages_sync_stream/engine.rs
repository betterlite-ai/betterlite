use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

use crate::api::messages_chat_output::records::{ChoiceRecord, UsageRecord};
use crate::api::messages_stream_usage::blocks::{
    delta_has_content, is_blank_delta, next_compaction_event, should_start_new_content_block,
    with_refusal_stop_details,
};
use crate::api::messages_stream_usage::error::StreamStateError;
use crate::api::messages_stream_usage::state::StreamState;
use crate::api::messages_stream_usage::usage::{
    augment_message_delta_usage, handle_choiceless_chunk, merge_usage_into_held_stop,
};

use super::frames;
use super::translate::{chunk_records, event as translate_event};

// What one call of __next__ does before it reads the provider: hand back buffered output, or pull a chunk.
pub enum Transition {
    Event(Object),
    Pull,
}

// The queued-event, message_start and compaction steps that run before the pull loop.
pub fn transition<S>(state: &mut StreamState<S>) -> Transition {
    if let Some(event) = state.chunk_queue.pop_front() {
        return Transition::Event(event);
    }
    if !state.message_progress.sent_first_chunk {
        state.message_progress.sent_first_chunk = true;
        return Transition::Event(frames::message_start(state));
    }
    if let Some(event) = next_compaction_event(state) {
        return Transition::Event(event);
    }
    Transition::Pull
}

// Whether the block machine opened a block, and whether it was the first one.
struct OpenBlock {
    starts: bool,
    opens_first: bool,
}

// The for-loop body for one pulled chunk; None means the chunk produced no output and the pull continues.
pub fn update<S>(
    state: &mut StreamState<S>,
    chunk: &StreamChunk,
    ids: &dyn IdSource,
) -> Result<Option<Object>, StreamStateError> {
    let (choices, usage) = chunk_records(chunk)?;
    if choices.is_empty() {
        return choiceless(state, usage.as_ref());
    }
    let Some(block) = open_block(state, &choices, ids)? else {
        return Ok(None);
    };
    let held_usage = state.holding_stop_reason_chunk.as_ref().and(usage.as_ref());
    // the merge path attaches applied_edits itself, so the translated frame carries none
    let final_chunk = choices
        .first()
        .is_some_and(|choice| choice.finish_reason.is_some());
    let edits = if final_chunk && held_usage.is_none() {
        state.applied_edits.clone()
    } else {
        Vec::new()
    };
    let translated = translate_event(state, chunk, &choices, usage.as_ref(), edits)?;
    let processed = with_refusal_stop_details(state, translated)?;
    if let Some(carried) = held_usage {
        return merge(state, carried);
    }
    if state.queued_usage_chunk {
        // usage already left the stream, so a trailing provider event would break Anthropic SSE order
        return Ok(None);
    }
    route(state, processed, block)
}

// A choiceless chunk only matters when a held stop is waiting for its usage.
fn choiceless<S>(
    state: &mut StreamState<S>,
    usage: Option<&UsageRecord>,
) -> Result<Option<Object>, StreamStateError> {
    if !handle_choiceless_chunk(state, usage)? {
        return Ok(None);
    }
    Ok(state.chunk_queue.pop_front())
}

// The block machine for one chunk: the queued start, or the index a new block takes.
fn open_block<S>(
    state: &mut StreamState<S>,
    choices: &[ChoiceRecord],
    ids: &dyn IdSource,
) -> Result<Option<OpenBlock>, StreamStateError> {
    let starts = should_start_new_content_block(state, choices, ids)?;
    let opens_first = !state.block_progress.sent_content_block_start;
    if opens_first && is_blank_delta(choices)? {
        return Ok(None);
    }
    if opens_first {
        state.block_progress.sent_content_block_start = true;
        state.block_progress.sent_content_block_finish = false;
        let start = frames::content_block_start(
            state.current_content_block_index,
            &state.current_content_block_start,
        );
        state.chunk_queue.push_back(start);
        return Ok(Some(OpenBlock {
            starts,
            opens_first,
        }));
    }
    if starts {
        state.current_content_block_index = state.current_content_block_index.saturating_add(1);
    }
    Ok(Some(OpenBlock {
        starts,
        opens_first,
    }))
}

// A usage-carrying chunk folds into the held stop, and nothing else leaves after it.
fn merge<S>(
    state: &mut StreamState<S>,
    carried: &UsageRecord,
) -> Result<Option<Object>, StreamStateError> {
    let merged = merge_usage_into_held_stop(state, carried)?;
    state.chunk_queue.push_back(merged);
    state.queued_usage_chunk = true;
    state.holding_stop_reason_chunk = None;
    Ok(state.chunk_queue.pop_front())
}

// Where the translated frame goes: a block transition, a block close, or a plain delta.
fn route<S>(
    state: &mut StreamState<S>,
    processed: Object,
    block: OpenBlock,
) -> Result<Option<Object>, StreamStateError> {
    if block.starts && !block.opens_first && !state.block_progress.sent_content_block_finish {
        return Ok(transition_events(state, processed));
    }
    let kind = processed.get("type").and_then(Json::as_str);
    if kind == Some("content_block_delta") && !delta_has_content(&processed) {
        // a tool block opens with empty arguments, so flush the queued start instead of waiting
        return Ok(state.chunk_queue.pop_front());
    }
    if kind == Some("message_delta") && !state.block_progress.sent_content_block_finish {
        return close_message(state, processed);
    }
    push_processed(state, processed)
}

// The trigger chunk carries the new block's first delta, so it is queued with the stop and start.
fn transition_events<S>(state: &mut StreamState<S>, processed: Object) -> Option<Object> {
    let stop = frames::block_frame(
        "content_block_stop",
        state.current_content_block_index.saturating_sub(1).max(0),
    );
    state.chunk_queue.push_back(stop);
    let start = frames::content_block_start(
        state.current_content_block_index,
        &state.current_content_block_start,
    );
    state.chunk_queue.push_back(start);
    if delta_has_content(&processed) {
        state.chunk_queue.push_back(processed);
    }
    state.block_progress.sent_content_block_finish = false;
    state.chunk_queue.pop_front()
}

// A message_delta closes the open block once, then waits for usage or goes out.
fn close_message<S>(
    state: &mut StreamState<S>,
    processed: Object,
) -> Result<Option<Object>, StreamStateError> {
    let stop = frames::block_frame("content_block_stop", state.current_content_block_index);
    state.chunk_queue.push_back(stop);
    state.block_progress.sent_content_block_finish = true;
    let holds_stop_reason = processed
        .get("delta")
        .and_then(Json::as_object)
        .and_then(|delta| delta.get("stop_reason"))
        .is_some_and(|reason| *reason != Json::Null);
    if holds_stop_reason {
        state.holding_stop_reason_chunk = Some(processed);
        return Ok(state.chunk_queue.pop_front());
    }
    let augmented = augment_message_delta_usage(state, processed)?;
    state.chunk_queue.push_back(augmented);
    Ok(state.chunk_queue.pop_front())
}

// An existing held chunk leaves before the processed event, and a message_delta takes the polyfill usage.
fn push_processed<S>(
    state: &mut StreamState<S>,
    processed: Object,
) -> Result<Option<Object>, StreamStateError> {
    if let Some(held) = state.holding_chunk.take() {
        state.chunk_queue.push_back(held);
    }
    if processed.get("type").and_then(Json::as_str) == Some("message_delta") {
        let augmented = augment_message_delta_usage(state, processed)?;
        state.chunk_queue.push_back(augmented);
        return Ok(state.chunk_queue.pop_front());
    }
    state.chunk_queue.push_back(processed);
    Ok(state.chunk_queue.pop_front())
}

// The chunk stream ended: the held content delta goes before the final message_delta, then one message_stop.
pub fn end_of_stream<S>(state: &mut StreamState<S>) -> Result<Option<Object>, StreamStateError> {
    if state.queued_usage_chunk {
        state.holding_chunk = None;
    } else {
        flush_held(state)?;
    }
    if !state.message_progress.sent_last_message {
        state.message_progress.sent_last_message = true;
        state.chunk_queue.push_back(frames::message_stop());
    }
    Ok(state.chunk_queue.pop_front())
}

// The held content delta outranks the block stop and the held message_delta.
fn flush_held<S>(state: &mut StreamState<S>) -> Result<(), StreamStateError> {
    if let Some(held) = state.holding_chunk.take() {
        state.chunk_queue.push_back(held);
    }
    let Some(held_stop) = state.holding_stop_reason_chunk.take() else {
        return Ok(());
    };
    if !state.block_progress.sent_content_block_finish {
        let stop = frames::block_frame("content_block_stop", state.current_content_block_index);
        state.chunk_queue.push_back(stop);
        state.block_progress.sent_content_block_finish = true;
    }
    let augmented = augment_message_delta_usage(state, held_stop)?;
    state.chunk_queue.push_back(augmented);
    Ok(())
}

// LiteLLM's StopIteration handler: queued events first, then a held stop, then the single message_stop.
pub fn stop_iteration<S>(state: &mut StreamState<S>) -> Result<Option<Object>, StreamStateError> {
    if let Some(event) = state.chunk_queue.pop_front() {
        return Ok(Some(event));
    }
    if let Some(held_stop) = state.holding_stop_reason_chunk.take() {
        return Ok(Some(release_held(state, held_stop)?));
    }
    if !state.message_progress.sent_last_message {
        state.message_progress.sent_last_message = true;
        return Ok(Some(frames::message_stop()));
    }
    Ok(None)
}

// A held stop whose block never closed queues its delta and hands back the missing stop first.
fn release_held<S>(
    state: &mut StreamState<S>,
    held_stop: Object,
) -> Result<Object, StreamStateError> {
    if state.block_progress.sent_content_block_finish {
        return augment_message_delta_usage(state, held_stop);
    }
    state.block_progress.sent_content_block_finish = true;
    let augmented = augment_message_delta_usage(state, held_stop)?;
    state.chunk_queue.push_back(augmented);
    Ok(frames::block_frame(
        "content_block_stop",
        state.current_content_block_index,
    ))
}

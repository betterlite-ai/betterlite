use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;

use crate::api::api_boundaries::ApiLimits;

use self::error::MessagesResponseEventError;
use self::input::ParsedEvent;
use self::log::StreamLog;

mod chunks;
pub mod error;
mod events;
pub mod failure;
pub mod input;
pub mod log;
mod tests;

// The "\n\n" litellm/llms/anthropic/pass_through/responses_adapters/transformation.py puts between reasoning summary parts.
const SUMMARY_PART_SEPARATOR: &str = "\n\n";

// A bool in the stream state; the newtype keeps struct_excessive_bools off a record that declares three
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct StreamFlag(pub bool);

// AnthropicResponsesStreamWrapper's queue and maps, which B986 iterates and re-emits as Anthropic SSE.
pub struct MessageStreamState {
    message_id: String,
    model: String,
    current_block_index: i64,
    item_id_to_block_index: BTreeMap<String, i64>,
    pending_tool_ids: BTreeMap<String, Json>,
    sent_message_start: StreamFlag,
    sent_message_stop: StreamFlag,
    stream_failed: StreamFlag,
    chunk_queue: VecDeque<Json>,
    refusal_text: String,
    logs: Vec<StreamLog>,
    redactor: Option<Arc<Redactor>>,
    limits: ApiLimits,
}

// The state as the oracle reads it after a step; every field is a wrapper attribute, a derived map or a queue effect.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MessageStreamSnapshot {
    pub chunks: Vec<Json>,
    pub current_block_index: i64,
    pub item_id_to_block_index: BTreeMap<String, i64>,
    pub pending_tool_ids: BTreeMap<String, Json>,
    pub refusal_text: String,
    pub sent_message_start: StreamFlag,
    pub sent_message_stop: StreamFlag,
    pub stream_failed: StreamFlag,
    pub logs: Vec<StreamLog>,
}

// The type's one inherent impl: multiple_inherent_impl keeps the event handlers free in events and failure.
impl MessageStreamState {
    #[must_use]
    pub const fn new(
        message_id: String,
        model: String,
        redactor: Option<Arc<Redactor>>,
        limits: ApiLimits,
    ) -> Self {
        Self {
            message_id,
            model,
            current_block_index: -1,
            item_id_to_block_index: BTreeMap::new(),
            pending_tool_ids: BTreeMap::new(),
            sent_message_start: StreamFlag(false),
            sent_message_stop: StreamFlag(false),
            stream_failed: StreamFlag(false),
            chunk_queue: VecDeque::new(),
            refusal_text: String::new(),
            logs: Vec::new(),
            redactor,
            limits,
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> MessageStreamSnapshot {
        MessageStreamSnapshot {
            chunks: self.chunk_queue.iter().cloned().collect(),
            current_block_index: self.current_block_index,
            item_id_to_block_index: self.item_id_to_block_index.clone(),
            pending_tool_ids: self.pending_tool_ids.clone(),
            refusal_text: self.refusal_text.clone(),
            sent_message_start: self.sent_message_start,
            sent_message_stop: self.sent_message_stop,
            stream_failed: self.stream_failed,
            logs: self.logs.clone(),
        }
    }

    // _process_event: one parsed Responses event becomes zero or more queued Anthropic chunks.
    pub fn process_event(
        &mut self,
        event: ParsedEvent<'_>,
    ) -> Result<(), MessagesResponseEventError> {
        let Some(event_type) = event.event_type() else {
            return Ok(());
        };
        match event_type {
            "response.created" => self.start_message(),
            "response.output_item.added" => events::item_added(self, event),
            "response.refusal.delta" => events::refusal_delta(self, event),
            "response.output_text.delta" => events::text_delta(self, event),
            "response.reasoning_summary_part.added" => events::summary_part_added(self, event),
            "response.reasoning_summary_text.delta" => events::summary_text_delta(self, event),
            "response.function_call_arguments.delta" => events::arguments_delta(self, event),
            "response.output_item.done" => events::item_done(self, event),
            "response.failed" => return failure::failed(self, event),
            "response.completed" | "response.incomplete" => return events::completed(self, event),
            _ => {}
        }
        Ok(())
    }

    pub(super) fn start_message(&mut self) {
        if self.sent_message_start.0 {
            return;
        }
        self.sent_message_start = StreamFlag(true);
        self.chunk_queue
            .push_back(chunks::message_start(&self.message_id, &self.model));
    }

    // Only a truthy id is registered; a truthy nonstring id can never match a later string lookup.
    pub(super) fn open_block(&mut self, item_id: Option<&Json>, content_block: Json) -> i64 {
        let block_idx = self.next_block_index();
        if let Some(Json::Text(text_id)) = item_id.filter(|value| input::is_present(value)) {
            self.item_id_to_block_index
                .insert(text_id.clone(), block_idx);
        }
        self.chunk_queue.push_back(chunks::block_frame(
            "content_block_start",
            block_idx,
            "content_block",
            content_block,
        ));
        block_idx
    }

    // A truthy id selects the map entry (-1 when unknown); a falsey or absent one selects the current block.
    pub(super) fn block_index_for(&self, item_id: Option<&Json>) -> i64 {
        match item_id.filter(|value| input::is_present(value)) {
            Some(Json::Text(_)) => self.mapped_block_index(item_id),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => -1,
            Some(Json::Null) | None => self.current_block_index,
        }
    }

    // The str-keyed lookup of _close_reasoning_item and the summary-part branch, which never fall back to the current block.
    pub(super) fn mapped_block_index(&self, item_id: Option<&Json>) -> i64 {
        let Some(text_id) = item_id.and_then(input::text) else {
            return -1;
        };
        self.item_id_to_block_index
            .get(text_id)
            .copied()
            .unwrap_or(-1)
    }

    // function_call_arguments.delta defaults the map lookup to the current block, as dict.get's default does.
    pub(super) fn arguments_block_index(&self, item_id: Option<&Json>) -> i64 {
        let found = self.block_index_for(item_id);
        if found >= 0 {
            return found;
        }
        self.current_block_index
    }

    const fn next_block_index(&mut self) -> i64 {
        self.current_block_index = self.current_block_index.saturating_add(1);
        self.current_block_index
    }
}

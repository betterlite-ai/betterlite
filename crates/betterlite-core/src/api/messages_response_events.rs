use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;

use crate::api::api_boundaries::ApiLimits;

use self::log::StreamLog;

mod chunks;
pub mod error;
mod events;
pub mod failure;
pub mod input;
pub mod log;
mod tests;

// litellm/llms/anthropic/pass_through/responses_adapters/transformation.py's REASONING_SUMMARY_PART_SEPARATOR
pub const REASONING_SUMMARY_PART_SEPARATOR: &str = "\n\n";

// AnthropicResponsesStreamWrapper's queue and maps, which B986 iterates and re-emits as Anthropic SSE.
pub struct MessageStreamState {
    message_id: String,
    model: String,
    current_block_index: i64,
    item_id_to_block_index: BTreeMap<String, i64>,
    pending_tool_ids: BTreeMap<String, Json>,
    sent_message_start: bool,
    sent_message_stop: bool,
    stream_failed: bool,
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
    pub sent_message_start: bool,
    pub sent_message_stop: bool,
    pub stream_failed: bool,
    pub logs: Vec<StreamLog>,
}

impl MessageStreamState {
    #[must_use]
    pub fn new(
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
            sent_message_start: false,
            sent_message_stop: false,
            stream_failed: false,
            chunk_queue: VecDeque::new(),
            refusal_text: String::new(),
            logs: Vec::new(),
            redactor,
            limits,
        }
    }

    // B986 polls this for one Anthropic chunk at a time.
    pub fn next_chunk(&mut self) -> Option<Json> {
        self.chunk_queue.pop_front()
    }

    #[must_use]
    pub const fn stream_failed(&self) -> bool {
        self.stream_failed
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

    pub(super) fn start_message(&mut self) {
        if self.sent_message_start {
            return;
        }
        self.sent_message_start = true;
        self.chunk_queue
            .push_back(chunks::message_start(&self.message_id, &self.model));
    }

    // Only a truthy id is registered; a truthy nonstring id can never match a later string lookup.
    pub(super) fn open_block(&mut self, item_id: Option<&Json>, content_block: Json) -> i64 {
        let block_idx = self.next_block_index();
        if let Some(Json::Text(text_id)) = item_id.filter(|value| input::is_truthy(value)) {
            self.item_id_to_block_index
                .insert(text_id.clone(), block_idx);
        }
        self.chunk_queue
            .push_back(chunks::content_block_start(block_idx, content_block));
        block_idx
    }

    // A truthy id selects the map entry (-1 when unknown); a falsey or absent one selects the current block.
    pub(super) fn block_index_for(&self, item_id: Option<&Json>) -> i64 {
        match item_id.filter(|value| input::is_truthy(value)) {
            Some(Json::Text(text_id)) => self
                .item_id_to_block_index
                .get(text_id)
                .copied()
                .unwrap_or(-1),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => -1,
            Some(Json::Null) | None => self.current_block_index,
        }
    }

    // The str-keyed lookup of _close_reasoning_item and the summary-part branch, which never fall back to the current block.
    pub(super) fn mapped_block_index(&self, item_id: Option<&Json>) -> i64 {
        match item_id.and_then(input::text) {
            Some(text_id) => self
                .item_id_to_block_index
                .get(text_id)
                .copied()
                .unwrap_or(-1),
            None => -1,
        }
    }

    fn next_block_index(&mut self) -> i64 {
        self.current_block_index = self.current_block_index.saturating_add(1);
        self.current_block_index
    }
}

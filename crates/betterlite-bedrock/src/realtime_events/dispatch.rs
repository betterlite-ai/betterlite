use betterlite_core::api::realtime_contracts::events::{
    AllDeltaType, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;

use super::decode::{json_dict, json_str};
use super::error::EventError;
use crate::agentcore_stream::IdSource;
use crate::realtime_completion::records::{
    DeltaChunk, DeltaType as CompletionDeltaType, ResponseState,
};
use crate::realtime_output::events::{DeltaType as OutputDeltaType, OutputIds};
use crate::realtime_session::RealtimeSession;

// A response's delta accumulator stays inside these caps in chunk count and text bytes; a long response fits both
pub(super) const MAX_DELTA_CHUNKS: usize = 4096;
pub(super) const MAX_DELTA_BYTES: usize = 0x0040_0000;

pub struct RealtimeEvents<'state> {
    pub session: &'state mut RealtimeSession,
    pub ids: &'state dyn IdSource,
    pub redactor: &'state Redactor,
}

// The seven fields transform_realtime_response hands back on every successful dispatch
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TransformState {
    pub(super) session_configuration_request: Option<String>,
    pub(super) output_item_id: Option<String>,
    pub(super) response_id: Option<String>,
    pub(super) delta_chunks: Option<Vec<Json>>,
    pub(super) item_chunks: Option<Vec<Json>>,
    pub(super) conversation_id: Option<String>,
    pub(super) delta_type: Option<AllDeltaType>,
}

impl From<&RealtimeResponseTransformInput> for TransformState {
    fn from(input: &RealtimeResponseTransformInput) -> Self {
        Self {
            session_configuration_request: input.session_configuration_request.clone(),
            output_item_id: input.current_output_item_id.clone(),
            response_id: input.current_response_id.clone(),
            delta_chunks: input.current_delta_chunks.clone(),
            item_chunks: input.current_item_chunks.clone(),
            conversation_id: input.current_conversation_id.clone(),
            delta_type: input.current_delta_type,
        }
    }
}

impl TransformState {
    pub(super) fn into_response(self, events: Vec<Json>) -> RealtimeResponseTypedDict {
        RealtimeResponseTypedDict {
            response: Json::List(events),
            current_output_item_id: self.output_item_id,
            current_response_id: self.response_id,
            current_delta_chunks: self.delta_chunks,
            current_conversation_id: self.conversation_id,
            current_item_chunks: self.item_chunks,
            current_delta_type: self.delta_type,
            session_configuration_request: self.session_configuration_request,
        }
    }

    pub(super) fn output_ids(&self) -> OutputIds {
        OutputIds {
            response: self.response_id.clone(),
            item: self.output_item_id.clone(),
            conversation: self.conversation_id.clone(),
        }
    }

    // LiteLLM tests the ids for truth, so an empty one is absent to every completion decision below
    pub(super) fn response_state(&self) -> ResponseState {
        ResponseState {
            output_item_id: self
                .output_item_id
                .as_deref()
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
            response_id: self
                .response_id
                .as_deref()
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
            conversation_id: self
                .conversation_id
                .as_deref()
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
            delta_type: self.delta_type.map(CompletionDeltaType::from),
            delta_chunks: self.delta_chunks.as_ref().map(|chunks| {
                chunks
                    .iter()
                    .map(|chunk| DeltaChunk {
                        delta: delta_text(chunk).to_owned(),
                    })
                    .collect::<Vec<_>>()
            }),
        }
    }

    // _response_done_events hands back no ids and no delta type; the conversation and chunks are the caller's
    pub(super) fn apply_done(&mut self, state: &ResponseState) {
        self.output_item_id.clone_from(&state.output_item_id);
        self.response_id.clone_from(&state.response_id);
        self.delta_type = state.delta_type.map(AllDeltaType::from);
    }
}

pub(super) fn delta_text(chunk: &Json) -> &str {
    json_dict(chunk)
        .and_then(|fields| fields.get("delta"))
        .and_then(json_str)
        .unwrap_or_default()
}

// A chunk is stored only while the accumulator stays inside both caps
pub(super) fn append_delta(state: &mut TransformState, chunk: Json) -> Result<(), EventError> {
    let count = state
        .delta_chunks
        .as_ref()
        .map_or(0, Vec::len)
        .saturating_add(1);
    let held: usize = state
        .delta_chunks
        .iter()
        .flatten()
        .map(|held_chunk| delta_text(held_chunk).len())
        .sum();
    check_delta_limits(count, held.saturating_add(delta_text(&chunk).len()))?;
    state.delta_chunks.get_or_insert_with(Vec::new).push(chunk);
    Ok(())
}

pub(super) const fn check_delta_limits(count: usize, bytes: usize) -> Result<(), EventError> {
    let (subject, limit) = if count > MAX_DELTA_CHUNKS {
        ("response delta chunks", MAX_DELTA_CHUNKS)
    } else if bytes > MAX_DELTA_BYTES {
        ("response delta bytes", MAX_DELTA_BYTES)
    } else {
        return Ok(());
    };
    Err(EventError::ExceedsLimit { subject, limit })
}

impl From<CompletionDeltaType> for AllDeltaType {
    fn from(value: CompletionDeltaType) -> Self {
        match value {
            CompletionDeltaType::Text => Self::Text,
            CompletionDeltaType::Audio => Self::Audio,
        }
    }
}

impl From<AllDeltaType> for CompletionDeltaType {
    fn from(value: AllDeltaType) -> Self {
        match value {
            AllDeltaType::Text => Self::Text,
            AllDeltaType::Audio => Self::Audio,
        }
    }
}

impl From<OutputDeltaType> for AllDeltaType {
    fn from(value: OutputDeltaType) -> Self {
        match value {
            OutputDeltaType::Text => Self::Text,
            OutputDeltaType::Audio => Self::Audio,
        }
    }
}

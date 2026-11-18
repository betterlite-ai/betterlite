use betterlite_core::api::realtime_contracts::events::{
    AllDeltaType, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;

use super::decode::{DecodedEvent, decode_realtime_native_event, json_dict, json_str};
use super::error::EventError;
use crate::agentcore_stream::IdSource;
use crate::realtime_completion::records::{
    DeltaChunk, DeltaType as CompletionDeltaType, ResponseState,
};
use crate::realtime_output::events::{DeltaType as OutputDeltaType, OutputIds};
use crate::realtime_session::RealtimeSession;

pub struct RealtimeEvents<'state> {
    pub session: &'state mut RealtimeSession,
    pub ids: &'state dyn IdSource,
    pub redactor: &'state Redactor,
}

impl RealtimeEvents<'_> {
    pub fn transform_realtime_response(
        &mut self,
        message: &[u8],
        input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, EventError> {
        let mut state = TransformState::from(input);
        let events = match decode_realtime_native_event(message)? {
            DecodedEvent::InvalidJson { preview } => {
                let preview = self.redactor.redact_string(&preview);
                tracing::warn!("Invalid JSON message: {preview}");
                Vec::new()
            }
            DecodedEvent::Event(event) => self.dispatch(event, &mut state)?,
        };
        Ok(state.into_response(events))
    }
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
            output_item_id: non_empty(self.output_item_id.as_deref()),
            response_id: non_empty(self.response_id.as_deref()),
            conversation_id: non_empty(self.conversation_id.as_deref()),
            delta_type: self.delta_type.map(CompletionDeltaType::from),
            delta_chunks: self
                .delta_chunks
                .as_ref()
                .map(|chunks| chunks.iter().map(delta_chunk).collect::<Vec<DeltaChunk>>()),
        }
    }

    // _response_done_events hands back no ids and no delta type; the conversation and chunks are the caller's
    pub(super) fn apply_done(&mut self, state: &ResponseState) {
        self.output_item_id = state.output_item_id.clone();
        self.response_id = state.response_id.clone();
        self.delta_type = state.delta_type.map(AllDeltaType::from);
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    let present = value.filter(|text| !text.is_empty());
    present.map(str::to_owned)
}

fn delta_chunk(chunk: &Json) -> DeltaChunk {
    let text = json_dict(chunk)
        .and_then(|fields| fields.get("delta"))
        .and_then(json_str)
        .unwrap_or_default();
    DeltaChunk {
        delta: text.to_owned(),
    }
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

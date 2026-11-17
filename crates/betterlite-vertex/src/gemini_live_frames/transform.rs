use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_core::api::responses_output_details::usage_details;
use betterlite_models::catalog::ModelCatalog;
use betterlite_prompts::arguments::json::Json;
use uuid::Uuid;

use super::decode::LiveFrame;
use super::error::LiveFrameError;
use super::events::{
    INPUT_TRANSCRIPT_COMPLETED, InputTranscriptEvent, OUTPUT_TRANSCRIPT_DELTA,
    OutputTranscriptEvent, event_json,
};
use super::ids::DrawnIds;
use super::state::{CallerState, FrameResponse};
use crate::gemini_live_accounting::response_done::{
    ResponseDoneInput, transform_response_done_event,
};
use crate::gemini_live_accounting::tool_calls::ToolCallMessage;
use crate::gemini_live_accounting::transcription::{
    TranscriptionUsage, consume_transcript_estimate,
};
use crate::gemini_live_client::records::Transcription;
use crate::gemini_live_client::state::LiveSessionState;
use crate::gemini_live_content::map::{MappedEvent, ModalityEvent, map_openai_event};
use crate::gemini_live_content::modality::handle_openai_modality_event;
use crate::gemini_live_content::start::{StartContent, start_content_events};
use crate::gemini_live_content::state::ContentState;
use crate::gemini_live_content::update::{ChunkKind, ChunkSource, update_current_chunks};
use crate::gemini_live_setup::created::session_created;
use crate::gemini_live_tool_turn::turn::{ToolTurnContext, ToolTurnInput, transform_tool_turn};

const KNOWN_KEYS: [&str; 3] = ["setupComplete", "serverContent", "toolCall"];
const MODEL_CONTENT_KEYS: [&str; 4] = [
    "modelTurn",
    "turnComplete",
    "interrupted",
    "generationComplete",
];

pub struct ResponseContext<'context> {
    pub model: &'context str,
    pub logging_session_id: &'context str,
    pub catalog: &'context ModelCatalog,
}

struct FrameState<'frame> {
    frame: &'frame mut LiveFrame,
    session: &'frame mut LiveSessionState,
    context: &'frame ResponseContext<'frame>,
    caller: CallerState,
    events: Vec<Json>,
    server_content_handled: bool,
    tool_call_handled: bool,
}

pub fn transform_realtime_response(
    frame: &mut LiveFrame,
    input: &CallerState,
    session: &mut LiveSessionState,
    context: &ResponseContext<'_>,
    uuids: &mut dyn FnMut() -> Uuid,
) -> Result<FrameResponse, LiveFrameError> {
    let mut work = FrameState {
        frame,
        session,
        context,
        caller: input.clone(),
        events: Vec::new(),
        server_content_handled: false,
        tool_call_handled: false,
    };
    work.transcribe(uuids)?;
    work.server_content_handled = work.transcription_only_content();
    work.dispatch_keys(uuids)?;
    work.finish()
}

impl FrameState<'_> {
    fn transcribe(&mut self, uuids: &mut dyn FnMut() -> Uuid) -> Result<(), LiveFrameError> {
        let Some(content) = self.frame.message.server_content.clone() else {
            return Ok(());
        };
        if let Some(text) = transcript_text(content.input_transcription.as_ref()) {
            let usage = consume_transcript_estimate(
                self.session,
                self.context.catalog,
                self.context.model,
            )?;
            let event = InputTranscriptEvent {
                event_type: INPUT_TRANSCRIPT_COMPLETED,
                event_id: format!("event_{}", uuids()),
                transcript: text,
                item_id: format!("item_{}", uuids()),
                content_index: 0,
                usage,
            };
            self.events.push(event_json(&event)?);
        }
        if let Some(text) = transcript_text(content.output_transcription.as_ref()) {
            self.push_output_transcript(&text, uuids)?;
        }
        Ok(())
    }

    fn push_output_transcript(
        &mut self,
        text: &str,
        uuids: &mut dyn FnMut() -> Uuid,
    ) -> Result<(), LiveFrameError> {
        let response_id = self.response_id(uuids);
        let item_id = match self.caller.output_item_id.as_deref() {
            Some(existing) => existing.to_owned(),
            None => self.start_audio_item(&response_id, uuids)?,
        };
        let event = OutputTranscriptEvent {
            event_type: OUTPUT_TRANSCRIPT_DELTA,
            event_id: format!("event_{}", uuids()),
            transcript: text.to_owned(),
            item_id,
            content_index: 0,
            output_index: 0,
            response_id,
            delta: text.to_owned(),
        };
        self.events.push(event_json(&event)?);
        Ok(())
    }

    fn response_id(&mut self, uuids: &mut dyn FnMut() -> Uuid) -> String {
        if let Some(existing) = self.caller.response_id.as_deref() {
            return existing.to_owned();
        }
        let generated = format!("resp_{}", uuids());
        self.caller.response_id = Some(generated.clone());
        generated
    }

    fn start_audio_item(
        &mut self,
        response_id: &str,
        uuids: &mut dyn FnMut() -> Uuid,
    ) -> Result<String, LiveFrameError> {
        let item_id = format!("item_{}", uuids());
        let conversation_id = self
            .caller
            .conversation_id
            .as_deref()
            .filter(|existing| !existing.is_empty())
            .map_or_else(|| format!("conv_{}", uuids()), str::to_owned);
        let events = start_content_events(
            &StartContent {
                setup_request: self.caller.session_configuration_request.as_deref(),
                response_id,
                output_item_id: &item_id,
                conversation_id: &conversation_id,
                delta_type: AllDeltaType::Audio,
            },
            &mut DrawnIds(uuids),
        )?;
        for event in &events {
            self.events.push(event_json(event)?);
        }
        self.caller.conversation_id = Some(conversation_id);
        self.caller.output_item_id = Some(item_id.clone());
        Ok(item_id)
    }

    fn transcription_only_content(&mut self) -> bool {
        let Some(Json::Object(content)) = self.frame.entries.get_mut("serverContent") else {
            return false;
        };
        if self.caller.delta_type.is_none() && !content.contains_key("modelTurn") {
            drop(content.shift_remove("generationComplete"));
        }
        !MODEL_CONTENT_KEYS
            .iter()
            .any(|key| content.contains_key(*key))
    }

    fn dispatch_keys(&mut self, uuids: &mut dyn FnMut() -> Uuid) -> Result<(), LiveFrameError> {
        let keys: Vec<String> = self.frame.entries.keys().cloned().collect();
        for key in keys {
            self.dispatch_one(&key, uuids)?;
        }
        Ok(())
    }

    fn dispatch_one(
        &mut self,
        key: &str,
        uuids: &mut dyn FnMut() -> Uuid,
    ) -> Result<(), LiveFrameError> {
        if !KNOWN_KEYS.contains(&key) {
            return Ok(());
        }
        if key == "serverContent" && self.server_content_handled {
            return Ok(());
        }
        let Some(value) = self.frame.entries.get(key).cloned() else {
            return Ok(());
        };
        match map_openai_event(key, &value, self.caller.delta_type)? {
            MappedEvent::SessionCreated => {
                let event = session_created(
                    self.context.model,
                    self.context.logging_session_id,
                    self.caller.session_configuration_request.as_deref(),
                    // the source writes the bare uuid here, with no event_ prefix
                    &uuids().to_string(),
                )?;
                self.events.push(event_json(&event)?);
            }
            MappedEvent::FunctionCallArgumentsDone => self.tool_turn(&value, uuids)?,
            MappedEvent::ResponseDone => self.close(uuids)?,
            MappedEvent::TextDelta => self.modality(ModalityEvent::TextDelta, uuids)?,
            MappedEvent::AudioDelta => self.modality(ModalityEvent::AudioDelta, uuids)?,
            MappedEvent::TextDone => self.modality(ModalityEvent::TextDone, uuids)?,
            MappedEvent::AudioDone => self.modality(ModalityEvent::AudioDone, uuids)?,
        }
        Ok(())
    }

    fn tool_turn(
        &mut self,
        value: &Json,
        uuids: &mut dyn FnMut() -> Uuid,
    ) -> Result<(), LiveFrameError> {
        let message = decode_tool_call(value)?;
        if message.function_calls.is_empty() {
            self.tool_call_handled = true;
            return Ok(());
        }
        let input = ToolTurnInput {
            conversation_id: self.caller.conversation_id.as_deref(),
            response_id: self.caller.response_id.as_deref(),
            output_item_id: self.caller.output_item_id.as_deref(),
            session_configuration_request: self.caller.session_configuration_request.as_deref(),
        };
        let turn = {
            let mut context = ToolTurnContext {
                state: &mut *self.session,
                frame: &mut self.frame.message,
                tool_call: &message,
                input: &input,
            };
            transform_tool_turn(&mut context, &|usage| usage_details(Some(usage)), uuids)?
        };
        for event in &turn.events {
            self.events.push(event_json(event)?);
        }
        self.caller.conversation_id = turn.conversation_id;
        self.caller.response_id = turn.response_id;
        self.caller.output_item_id = turn.output_item_id;
        Ok(())
    }

    fn modality(
        &mut self,
        mapped: ModalityEvent,
        uuids: &mut dyn FnMut() -> Uuid,
    ) -> Result<(), LiveFrameError> {
        let mut content = ContentState {
            output_item_id: self.caller.output_item_id.clone(),
            response_id: self.caller.response_id.clone(),
            conversation_id: self.caller.conversation_id.clone(),
            delta_chunks: self.caller.delta_chunks.clone(),
            item_chunks: self.caller.item_chunks.clone(),
            delta_type: self.caller.delta_type,
        };
        let frame = Json::Object(self.frame.entries.clone());
        let events = handle_openai_modality_event(
            &mut content,
            mapped,
            &frame,
            self.caller.session_configuration_request.as_deref(),
            &mut DrawnIds(uuids),
        )?;
        for event in &events {
            self.events.push(event_json(event)?);
        }
        self.caller
            .output_item_id
            .clone_from(&content.output_item_id);
        self.caller.response_id.clone_from(&content.response_id);
        self.caller
            .conversation_id
            .clone_from(&content.conversation_id);
        self.caller.delta_type = content.delta_type;
        Ok(())
    }

    fn close(&mut self, uuids: &mut dyn FnMut() -> Uuid) -> Result<(), LiveFrameError> {
        if self.caller.response_id.is_none() && self.pending_function_call() {
            self.suppress();
            return Ok(());
        }
        // the source hands the done transform a shallow copy, so only a tool turn drains the frame's own usage
        let mut copy = self.frame.message.clone();
        let input = ResponseDoneInput {
            response_id: self.caller.response_id.as_deref(),
            conversation_id: self.caller.conversation_id.as_deref(),
            output_items: &[],
            session_configuration_request: self.caller.session_configuration_request.as_deref(),
        };
        let event = transform_response_done_event(
            self.session,
            &mut copy,
            &input,
            &|usage| usage_details(Some(usage)),
            uuids,
        )?;
        self.events.push(event_json(&event)?);
        self.caller.output_item_id = None;
        self.caller.response_id = None;
        Ok(())
    }

    fn suppress(&mut self) {
        if let Some(usage) = self.frame.message.usage_metadata.clone() {
            self.session.pending_usage_metadata = Some(usage);
        }
        self.server_content_handled = true;
    }

    fn pending_function_call(&self) -> bool {
        self.caller.item_chunks.as_deref().is_some_and(|chunks| {
            chunks.iter().any(|chunk| {
                chunk
                    .as_object()
                    .and_then(|fields| fields.get("item"))
                    .and_then(Json::as_object)
                    .and_then(|item| item.get("type"))
                    .and_then(Json::as_str)
                    .is_some_and(|kind| kind == "function_call")
            })
        })
    }

    fn finish(mut self) -> Result<FrameResponse, LiveFrameError> {
        if self.events.is_empty() {
            if let Some(usage) = self.frame.message.usage_metadata.clone() {
                self.session.pending_usage_metadata = Some(usage);
            }
            let unhandled = self.frame.entries.keys().any(|key| {
                KNOWN_KEYS.contains(&key.as_str())
                    && !(key == "serverContent" && self.server_content_handled)
                    && !(key == "toolCall" && self.tool_call_handled)
            });
            if unhandled {
                return Err(LiveFrameError::UnknownMessage);
            }
            return Ok(FrameResponse {
                events: Vec::new(),
                state: self.caller,
            });
        }
        self.caller.delta_chunks = update_current_chunks(
            ChunkSource::List(&self.events),
            self.caller.delta_chunks.take(),
            ChunkKind::Delta,
        );
        self.caller.item_chunks = update_current_chunks(
            ChunkSource::List(&self.events),
            self.caller.item_chunks.take(),
            ChunkKind::Item,
        );
        Ok(FrameResponse {
            events: self.events,
            state: self.caller,
        })
    }
}

fn transcript_text(transcription: Option<&Transcription>) -> Option<String> {
    transcription
        .and_then(|event| event.text.as_deref())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn decode_tool_call(value: &Json) -> Result<ToolCallMessage, LiveFrameError> {
    let encoded = serde_json::to_vec(value).map_err(|source| LiveFrameError::InvalidField {
        path: "toolCall".to_owned(),
        source,
    })?;
    serde_json::from_slice(&encoded).map_err(|source| LiveFrameError::InvalidField {
        path: "toolCall".to_owned(),
        source,
    })
}

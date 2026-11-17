use betterlite_core::api::responses_output_details::usage_details;
use betterlite_models::catalog::ModelCatalog;
use betterlite_prompts::arguments::json::Json;
use uuid::Uuid;

use super::decode::LiveFrame;
use super::error::LiveFrameError;
use super::events::event_json;
use super::ids::DrawnIds;
use super::state::{CallerState, FrameResponse};
use super::transcript::{transcribe, transcription_only_content};
use crate::gemini_live_accounting::response_done::{
    ResponseDoneInput, transform_response_done_event,
};
use crate::gemini_live_accounting::tool_calls::ToolCallMessage;
use crate::gemini_live_client::state::LiveSessionState;
use crate::gemini_live_content::map::{MappedEvent, ModalityEvent, map_openai_event};
use crate::gemini_live_content::modality::handle_openai_modality_event;
use crate::gemini_live_content::state::ContentState;
use crate::gemini_live_content::update::{ChunkKind, ChunkSource, update_current_chunks};
use crate::gemini_live_setup::created::session_created;
use crate::gemini_live_tool_turn::turn::{ToolTurnContext, ToolTurnInput, transform_tool_turn};

const KNOWN_KEYS: [&str; 3] = ["setupComplete", "serverContent", "toolCall"];

pub struct ResponseContext<'context> {
    pub model: &'context str,
    pub logging_session_id: &'context str,
    pub catalog: &'context ModelCatalog,
}

pub(super) struct FrameState<'frame> {
    pub(super) frame: &'frame mut LiveFrame,
    pub(super) session: &'frame mut LiveSessionState,
    pub(super) context: &'frame ResponseContext<'frame>,
    pub(super) caller: CallerState,
    pub(super) events: Vec<Json>,
    pub(super) server_content_handled: bool,
    pub(super) tool_call_handled: bool,
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
    transcribe(&mut work, uuids)?;
    let handled = transcription_only_content(&mut work);
    work.server_content_handled = handled;
    work.dispatch_keys(uuids)?;
    work.finish()
}

impl FrameState<'_> {
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
            let unhandled = self.frame.entries.keys().any(|key| match key.as_str() {
                "serverContent" => !self.server_content_handled,
                "toolCall" => !self.tool_call_handled,
                _ => KNOWN_KEYS.contains(&key.as_str()),
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

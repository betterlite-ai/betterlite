use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::decode::{NativeEvent, require_object};
use super::dispatch::{RealtimeEvents, TransformState, append_delta};
use super::error::EventError;
use crate::realtime_completion::content_end::content_end_events;
use crate::realtime_completion::tool_input::ToolUseEvent;
use crate::realtime_completion::tool_use::tool_use_events;
use crate::realtime_completion::usage::response_done_events;
use crate::realtime_output::NovaRealtimeOutput;
use crate::realtime_output::events::{AudioOutput, ContentStart, ContentStarted, TextOutput};
use crate::realtime_session::records::{BedrockContentEnd, UsageEvent};

impl RealtimeEvents<'_> {
    pub(super) fn dispatch(
        &mut self,
        event: NativeEvent,
        state: &mut TransformState,
    ) -> Result<Vec<Json>, EventError> {
        match event {
            NativeEvent::SessionStart => {
                // LiteLLM marks the session update as sent with json.dumps({"configured": True}) and never reads it back
                let marker = IndexMap::from([("configured".to_owned(), Json::Bool(true))]);
                state.session_configuration_request = Some(python_json::dumps_object(&marker));
                Ok(Vec::new())
            }
            NativeEvent::ContentStart(content_start) => self.content_start(state, &content_start),
            NativeEvent::TextOutput(text) => self.text_output(state, &text),
            NativeEvent::AudioOutput(audio) => self.audio_output(state, &audio),
            NativeEvent::ContentEnd(raw) => self.content_end(state, &raw),
            NativeEvent::UserSpeechStart => self.user_speech(true),
            NativeEvent::UserSpeechEnd => self.user_speech(false),
            NativeEvent::UsageEvent(usage) => {
                self.usage(usage);
                Ok(Vec::new())
            }
            NativeEvent::ToolUse(tool_use) => self.tool_use(state, &tool_use),
            NativeEvent::PromptEnd => self.finish_response(state),
            NativeEvent::Unknown => Ok(Vec::new()),
        }
    }

    fn content_start(
        &mut self,
        state: &mut TransformState,
        content_start: &ContentStart,
    ) -> Result<Vec<Json>, EventError> {
        let current = state.output_ids();
        let mut output = NovaRealtimeOutput {
            session: &mut *self.session,
            ids: self.ids,
        };
        let started = output.transform_content_start_event(content_start, &current);
        let ContentStarted {
            events,
            ids,
            delta_type,
        } = started;
        state.response_id = ids.response;
        state.output_item_id = ids.item;
        state.conversation_id = ids.conversation;
        state.delta_type = delta_type.map(AllDeltaType::from);
        events_json(&events)
    }

    // A user transcript active means the textOutput is ASR, not assistant text
    fn text_output(
        &mut self,
        state: &mut TransformState,
        text: &TextOutput,
    ) -> Result<Vec<Json>, EventError> {
        if self.session.transcript.active {
            let mut output = NovaRealtimeOutput {
                session: &mut *self.session,
                ids: self.ids,
            };
            let event = output.transform_user_transcript_event(&text.content);
            let json = event_json(&event)?;
            return Ok(vec![json]);
        }
        let current = state.output_ids();
        let mut discarded = None;
        let output = NovaRealtimeOutput {
            session: &mut *self.session,
            ids: self.ids,
        };
        let Some(delta) = output.transform_text_output_event(text, &current, &mut discarded) else {
            return Ok(Vec::new());
        };
        let json = event_json(&delta)?;
        append_delta(state, json.clone())?;
        Ok(vec![json])
    }

    fn audio_output(
        &mut self,
        state: &TransformState,
        audio: &AudioOutput,
    ) -> Result<Vec<Json>, EventError> {
        let current = state.output_ids();
        let Some(delta) = NovaRealtimeOutput {
            session: &mut *self.session,
            ids: self.ids,
        }
        .transform_audio_output_event(audio, &current) else {
            return Ok(Vec::new());
        };
        let json = event_json(&delta)?;
        Ok(vec![json])
    }

    fn user_speech(&mut self, is_start: bool) -> Result<Vec<Json>, EventError> {
        let mut output = NovaRealtimeOutput {
            session: &mut *self.session,
            ids: self.ids,
        };
        let event = output.transform_user_speech_event(is_start);
        let json = event_json(&event)?;
        Ok(vec![json])
    }

    // A contentEnd that closes a user transcript is never parsed for a stop reason
    fn content_end(
        &mut self,
        state: &mut TransformState,
        raw: &str,
    ) -> Result<Vec<Json>, EventError> {
        if self.session.transcript.active {
            return self.user_transcript_end();
        }
        self.assistant_content_end(state, raw)
    }

    fn user_transcript_end(&mut self) -> Result<Vec<Json>, EventError> {
        self.session.transcript.active = false;
        self.session.transcript.generation_stage = None;
        let mut output = NovaRealtimeOutput {
            session: &mut *self.session,
            ids: self.ids,
        };
        let Some(event) = output.user_transcript_completed_events() else {
            return Ok(Vec::new());
        };
        let json = event_json(&event)?;
        Ok(vec![json])
    }

    fn assistant_content_end(
        &mut self,
        state: &mut TransformState,
        raw: &str,
    ) -> Result<Vec<Json>, EventError> {
        let mut response_state = state.response_state();
        let done = content_end_events(&mut response_state, self.ids);
        let mut events = events_json(&done)?;
        require_object(raw, "contentEnd")?;
        let end: BedrockContentEnd =
            serde_json::from_str(raw).map_err(|source| EventError::Payload {
                field: "contentEnd",
                source,
            })?;
        // A content end that closed a response resets its accumulator; one without ids leaves it as it came
        if response_state.output_item_id.is_some() && response_state.response_id.is_some() {
            state.delta_chunks = None;
        }
        if end.stop_reason.as_deref() == Some("END_TURN") {
            events.extend(self.finish_response(state)?);
        }
        Ok(events)
    }

    fn finish_response(&mut self, state: &mut TransformState) -> Result<Vec<Json>, EventError> {
        let mut response_state = state.response_state();
        let done = response_done_events(&mut *self.session, &mut response_state, self.ids)?;
        state.apply_done(&response_state);
        events_json(&done)
    }

    fn tool_use(
        &mut self,
        state: &TransformState,
        event: &ToolUseEvent,
    ) -> Result<Vec<Json>, EventError> {
        let response_state = state.response_state();
        let output = tool_use_events(&response_state, event, self.ids)?;
        tracing::debug!(
            "Tool use event: {} (ID: {})",
            output.tool_name,
            output.tool_call_id
        );
        events_json(&output.events)
    }

    fn usage(&mut self, usage: UsageEvent) {
        let mut output = NovaRealtimeOutput {
            session: &mut *self.session,
            ids: self.ids,
        };
        output.transform_usage_event(usage);
    }
}

fn event_json<T>(event: &T) -> Result<Json, EventError>
where
    T: Serialize,
{
    serde_json::from_value(
        serde_json::to_value(event).map_err(|source| EventError::Encode { source })?,
    )
    .map_err(|source| EventError::Encode { source })
}

fn events_json<T>(events: &[T]) -> Result<Vec<Json>, EventError>
where
    T: Serialize,
{
    let mut encoded = Vec::with_capacity(events.len());
    for event in events {
        encoded.push(event_json(event)?);
    }
    Ok(encoded)
}

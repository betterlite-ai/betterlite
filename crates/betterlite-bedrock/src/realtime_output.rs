pub mod events;
pub mod metadata;

mod tests;

use std::mem;

use crate::agentcore_stream::IdSource;
use crate::realtime_session::RealtimeSession;
use crate::realtime_session::records::UsageEvent;
use events::{
    AudioOutput, ContentPart, ContentPartAdded, ContentStart, ContentStarted, DeltaType,
    OutputEvent, OutputIds, OutputItem, OutputItemAdded, ResponseCreated, ResponseDelta,
    ResponseObject, SpeechBoundary, TextOutput, TranscriptionCompleted, TranscriptionDelta,
};
use metadata::parse_generation_stage;

pub struct NovaRealtimeOutput<'state> {
    pub session: &'state mut RealtimeSession,
    pub ids: &'state dyn IdSource,
}

struct ResolvedIds {
    response: String,
    item: String,
    conversation: String,
}

impl NovaRealtimeOutput<'_> {
    pub fn transform_content_start_event(
        &mut self,
        content_start: &ContentStart,
        current: &OutputIds,
    ) -> ContentStarted {
        if content_start.role.as_deref() == Some("ASSISTANT") {
            return self.assistant_content_start(content_start, current);
        }
        if content_start.role.as_deref() == Some("USER")
            && content_start.content_type.as_deref() == Some("TEXT")
        {
            self.session.transcript.active = true;
            self.session.transcript.generation_stage =
                parse_generation_stage(content_start.additional_model_fields.as_ref());
        }
        ContentStarted {
            events: Vec::new(),
            ids: current.clone(),
            delta_type: None,
        }
    }

    // LiteLLM fills any empty id but sends response.created only when the response id is absent
    fn assistant_content_start(
        &self,
        content_start: &ContentStart,
        current: &OutputIds,
    ) -> ContentStarted {
        tracing::debug!("Handling ASSISTANT contentStart");
        let ids = ResolvedIds {
            response: self.id_or_new(current.response.as_deref(), "resp"),
            item: self.id_or_new(current.item.as_deref(), "item"),
            conversation: self.id_or_new(current.conversation.as_deref(), "conv"),
        };
        let delta_type = match content_start.content_type.as_deref().unwrap_or("TEXT") {
            "TEXT" => DeltaType::Text,
            _ => DeltaType::Audio,
        };
        let events = self.response_events(&ids, current.response.is_none(), delta_type);
        ContentStarted {
            events,
            ids: OutputIds {
                response: Some(ids.response),
                item: Some(ids.item),
                conversation: Some(ids.conversation),
            },
            delta_type: Some(delta_type),
        }
    }

    fn response_events(
        &self,
        ids: &ResolvedIds,
        is_new_response: bool,
        delta_type: DeltaType,
    ) -> Vec<OutputEvent> {
        let mut events = Vec::with_capacity(3);
        let created = ResponseCreated {
            event_id: format!("event_{}", self.ids.next_id()),
            response: ResponseObject {
                object: "realtime.response",
                id: ids.response.clone(),
                status: "in_progress",
                output: Vec::new(),
                conversation_id: ids.conversation.clone(),
            },
        };
        if is_new_response {
            events.push(OutputEvent::ResponseCreated(created));
        }
        events.push(OutputEvent::OutputItemAdded(OutputItemAdded {
            response_id: ids.response.clone(),
            output_index: 0,
            item: OutputItem {
                id: ids.item.clone(),
                object: "realtime.item",
                item_type: "message",
                status: "in_progress",
                role: "assistant",
                content: Vec::new(),
            },
        }));
        let part = match delta_type {
            DeltaType::Text => ContentPart::Text {
                text: String::new(),
            },
            DeltaType::Audio => ContentPart::Audio {
                transcript: String::new(),
            },
        };
        events.push(OutputEvent::ContentPartAdded(ContentPartAdded {
            content_index: 0,
            output_index: 0,
            event_id: format!("event_{}", self.ids.next_id()),
            item_id: ids.item.clone(),
            part,
            response_id: ids.response.clone(),
        }));
        events
    }

    fn id_or_new(&self, current: Option<&str>, prefix: &str) -> String {
        current
            .filter(|id| !id.is_empty())
            .map_or_else(|| format!("{prefix}_{}", self.ids.next_id()), str::to_owned)
    }

    fn current_user_item_id(&mut self, new_utterance: bool) -> String {
        match self.session.transcript.item_id.clone() {
            Some(existing) if !new_utterance => existing,
            Some(_) | None => {
                let fresh = format!("item_{}", self.ids.next_id());
                self.session.transcript.item_id = Some(fresh.clone());
                fresh
            }
        }
    }

    pub fn transform_user_speech_event(&mut self, is_speech_start: bool) -> OutputEvent {
        tracing::debug!(is_speech_start, "Handling userSpeech boundary");
        let boundary = SpeechBoundary {
            event_id: format!("event_{}", self.ids.next_id()),
            item_id: self.current_user_item_id(is_speech_start),
        };
        if is_speech_start {
            OutputEvent::SpeechStarted(boundary)
        } else {
            OutputEvent::SpeechStopped(boundary)
        }
    }

    pub fn transform_usage_event(&mut self, usage: UsageEvent) {
        tracing::debug!("Handling usageEvent");
        self.session.cumulative_usage = usage;
    }

    pub fn transform_user_transcript_event(&mut self, transcript: &str) -> OutputEvent {
        tracing::debug!("Handling USER textOutput (ASR transcript)");
        let delta = TranscriptionDelta {
            event_id: format!("event_{}", self.ids.next_id()),
            item_id: self.current_user_item_id(false),
            content_index: 0,
            delta: transcript.to_owned(),
        };
        if self.session.transcript.generation_stage.as_deref() != Some("SPECULATIVE") {
            self.session.transcript.buffer.push_str(transcript);
        }
        OutputEvent::TranscriptionDelta(delta)
    }

    pub fn user_transcript_completed_events(&mut self) -> Option<OutputEvent> {
        if self.session.transcript.buffer.is_empty() {
            return None;
        }
        let transcript = mem::take(&mut self.session.transcript.buffer);
        Some(OutputEvent::TranscriptionCompleted(
            TranscriptionCompleted {
                event_id: format!("event_{}", self.ids.next_id()),
                item_id: self.current_user_item_id(false),
                content_index: 0,
                transcript,
            },
        ))
    }

    pub fn transform_text_output_event(
        &self,
        text_output: &TextOutput,
        current: &OutputIds,
        delta_chunks: &mut Option<Vec<ResponseDelta>>,
    ) -> Option<OutputEvent> {
        tracing::debug!("Handling textOutput");
        let delta = self.response_delta(current, &text_output.content)?;
        delta_chunks
            .get_or_insert_with(Vec::new)
            .push(delta.clone());
        Some(OutputEvent::TextDelta(delta))
    }

    #[must_use]
    pub fn transform_audio_output_event(
        &self,
        audio_output: &AudioOutput,
        current: &OutputIds,
    ) -> Option<OutputEvent> {
        tracing::debug!("Handling audioOutput");
        self.response_delta(current, &audio_output.content)
            .map(OutputEvent::AudioDelta)
    }

    fn response_delta(&self, current: &OutputIds, content: &str) -> Option<ResponseDelta> {
        let item_id = current.item.as_deref().filter(|id| !id.is_empty())?;
        let response_id = current.response.as_deref().filter(|id| !id.is_empty())?;
        Some(ResponseDelta {
            content_index: 0,
            event_id: format!("event_{}", self.ids.next_id()),
            item_id: item_id.to_owned(),
            output_index: 0,
            response_id: response_id.to_owned(),
            delta: content.to_owned(),
        })
    }
}

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::realtime_session::RealtimeSession;
use crate::realtime_session::records::{
    AudioOutputConfiguration, ContentStart, PromptStart, SessionStart, UpdateFrame,
};
use crate::realtime_session::update::SessionUpdate;

pub(super) const PROMPT: &str = "00000000-0000-4000-8000-000000000001";
pub(super) const CONTENT: &str = "00000000-0000-4000-8000-000000000002";
pub(super) const AUDIO_CONTENT: &str = "00000000-0000-4000-8000-000000000003";
pub(super) const INSTRUCTION: &str = "00000000-0000-4000-8000-000000000004";
pub(super) const CREATED: &str = "00000000-0000-4000-8000-000000000011";
pub(super) const UPDATED: &str = "00000000-0000-4000-8000-000000000012";
pub(super) const UPDATED_EMPTY: &str = "00000000-0000-4000-8000-000000000013";
pub(super) const UPDATED_EXPLICIT: &str = "00000000-0000-4000-8000-000000000014";
pub(super) const MODEL_EVENT: &str = "00000000-0000-4000-8000-000000000015";
pub(super) const TRACE_ID: &str = "trace-realtime-1";

pub(super) struct FixedIds {
    values: Mutex<VecDeque<Uuid>>,
}

impl FixedIds {
    pub(super) fn new(values: &[&str]) -> Self {
        Self {
            values: Mutex::new(
                values
                    .iter()
                    .map(|value| Uuid::parse_str(value).expect("a fixture uuid"))
                    .collect(),
            ),
        }
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or(Uuid::nil())
    }
}

pub(super) fn session(values: &[&str]) -> RealtimeSession {
    RealtimeSession::new(&FixedIds::new(values))
}

pub(super) fn update(text: &str) -> SessionUpdate {
    serde_json::from_str(text).expect("a session.update fixture")
}

pub(super) fn apply(state: &mut RealtimeSession, text: &str, ids: &[&str]) -> Vec<UpdateFrame> {
    let fixture = update(text);
    let ids = FixedIds::new(ids);
    state.apply_update(&fixture, &ids)
}

pub(super) fn session_start(frames: &[UpdateFrame]) -> &SessionStart {
    match frames.first() {
        Some(UpdateFrame::SessionStart(frame)) => &frame.event.session_start,
        _ => panic!("the first frame is sessionStart"),
    }
}

pub(super) fn prompt_start(frames: &[UpdateFrame]) -> &PromptStart {
    match frames.get(1) {
        Some(UpdateFrame::PromptStart(frame)) => &frame.event.prompt_start,
        _ => panic!("the second frame is promptStart"),
    }
}

pub(super) fn output_audio(frames: &[UpdateFrame]) -> &AudioOutputConfiguration {
    &prompt_start(frames).audio_output_configuration
}

pub(super) fn content_start(frames: &[UpdateFrame]) -> &ContentStart {
    match frames.get(2) {
        Some(UpdateFrame::ContentStart(frame)) => &frame.event.content_start,
        _ => panic!("the third frame is contentStart"),
    }
}

pub(super) fn text_input(frames: &[UpdateFrame]) -> (&str, &str, &str) {
    match frames.get(3) {
        Some(UpdateFrame::TextInput(frame)) => (
            &frame.event.text_input.prompt_name,
            &frame.event.text_input.content_name,
            &frame.event.text_input.content,
        ),
        _ => panic!("the fourth frame is textInput"),
    }
}

pub(super) fn content_end(frames: &[UpdateFrame]) -> (&str, &str) {
    match frames.get(4) {
        Some(UpdateFrame::ContentEnd(frame)) => (
            &frame.event.content_end.prompt_name,
            &frame.event.content_end.content_name,
        ),
        _ => panic!("the fifth frame is contentEnd"),
    }
}

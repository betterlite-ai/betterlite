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
pub(super) const NO_MODEL: &str = "00000000-0000-4000-8000-000000000016";
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
    let source = FixedIds::new(ids);
    state.apply_update(&fixture, &source)
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

pub(super) struct InstructionContent<'a> {
    pub content_start: &'a ContentStart,
    pub text_input: (&'a str, &'a str, &'a str),
    pub content_end: (&'a str, &'a str),
}

pub(super) fn instruction_content(frames: &[UpdateFrame]) -> InstructionContent<'_> {
    let Some(UpdateFrame::ContentStart(start)) = frames.get(2) else {
        panic!("the third frame is contentStart");
    };
    let Some(UpdateFrame::TextInput(input)) = frames.get(3) else {
        panic!("the fourth frame is textInput");
    };
    let Some(UpdateFrame::ContentEnd(end)) = frames.get(4) else {
        panic!("the fifth frame is contentEnd");
    };
    let content_start = &start.event.content_start;
    let text_input = &input.event.text_input;
    let content_end = &end.event.content_end;
    InstructionContent {
        content_start,
        text_input: (
            text_input.prompt_name.as_str(),
            text_input.content_name.as_str(),
            text_input.content.as_str(),
        ),
        content_end: (
            content_end.prompt_name.as_str(),
            content_end.content_name.as_str(),
        ),
    }
}

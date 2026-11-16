use std::io;
use std::sync::{Arc, Mutex};

use tracing::subscriber::{DefaultGuard, set_default};
use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::realtime_input::RealtimeInput;
use crate::realtime_input::error::InputError;
use crate::realtime_input::frames::InputFrame;
use crate::realtime_session::RealtimeSession;

pub(super) const PROMPT: &str = "00000000-0000-0000-0000-000000000001";
pub(super) const AUDIO: &str = "00000000-0000-0000-0000-000000000003";

#[derive(Default)]
pub(super) struct CountingIds(Mutex<u128>);

impl IdSource for CountingIds {
    fn next_id(&self) -> Uuid {
        let mut last = self.0.lock().unwrap();
        *last = last.saturating_add(1);
        Uuid::from_u128(*last)
    }
}

pub(super) fn id(value: u128) -> String {
    Uuid::from_u128(value).to_string()
}

pub(super) fn new_input() -> (RealtimeInput, CountingIds) {
    let ids = CountingIds::default();
    let session = RealtimeSession::new(&ids);
    (
        RealtimeInput {
            session,
            open_audio: None,
        },
        ids,
    )
}

pub(super) fn try_send(
    input: &mut RealtimeInput,
    ids: &CountingIds,
    message: &str,
) -> Result<Vec<String>, InputError> {
    let frames = input.transform_realtime_request(message.as_bytes(), ids)?;
    Ok(wire(&frames))
}

pub(super) fn wire(frames: &[InputFrame]) -> Vec<String> {
    frames
        .iter()
        .map(|frame| serde_json::to_string(frame).unwrap())
        .collect()
}

pub(super) fn send(input: &mut RealtimeInput, ids: &CountingIds, message: &str) -> Vec<String> {
    try_send(input, ids, message).unwrap()
}

pub(super) fn audio_start(content: &str, rate: &str) -> String {
    format!(
        r#"{{"event":{{"contentStart":{{"promptName":"{PROMPT}","contentName":"{content}","type":"AUDIO","interactive":true,"role":"USER","audioInputConfiguration":{{"mediaType":"audio/lpcm","sampleRateHertz":{rate},"sampleSizeBits":16,"channelCount":1,"audioType":"SPEECH","encoding":"base64"}}}}}}}}"#
    )
}

pub(super) fn audio_input(content: &str, data: &str) -> String {
    format!(
        r#"{{"event":{{"audioInput":{{"promptName":"{PROMPT}","contentName":"{content}","content":{data}}}}}}}"#
    )
}

pub(super) fn content_end(content: &str) -> String {
    format!(r#"{{"event":{{"contentEnd":{{"promptName":"{PROMPT}","contentName":"{content}"}}}}}}"#)
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

pub(super) fn prompt_end() -> String {
    format!(r#"{{"event":{{"promptEnd":{{"promptName":"{PROMPT}"}}}}}}"#)
}

pub(super) const SESSION_END: &str = r#"{"event":{"sessionEnd":{}}}"#;

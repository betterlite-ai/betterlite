use std::collections::VecDeque;
use std::mem;

use betterlite_logging::passthrough_stream_logging::sse_frames::split_complete_sse_frames;
use tokio::sync::mpsc::Receiver;

use super::StreamEvent;
use crate::cato::error::ModelStreamError;

pub const MAX_FIRST_SSE_FRAME_BYTES: usize = 64 * 1024;

const FRAME_ENDS: [&[u8]; 3] = [b"\r\n\r\n", b"\n\n", b"\r\r"];

// A leading frame with no data line is relayed as a preface, everything else is an ordinary event
#[derive(Debug, Clone)]
pub enum Coalesced {
    Preface(Vec<u8>),
    Event(StreamEvent),
}

impl Coalesced {
    #[must_use]
    pub fn into_event(self) -> StreamEvent {
        match self {
            Self::Preface(raw) => StreamEvent::Bytes(raw),
            Self::Event(event) => event,
        }
    }
}

// Splits complete frames into (the frames before the first one holding a data line, that frame and the rest)
#[must_use]
pub fn split_sse_preface(complete_frames: &[u8]) -> (&[u8], &[u8]) {
    let mut start = 0_usize;
    let mut cursor = 0_usize;
    while let Some(tail) = complete_frames.get(cursor..) {
        let Some(length) = (0..tail.len()).find_map(|offset| {
            let rest = tail.get(offset..)?;
            let end = FRAME_ENDS.iter().find(|end| rest.starts_with(end))?;
            offset.checked_add(end.len())
        }) else {
            break;
        };
        cursor = cursor.saturating_add(length);
        let frame = complete_frames.get(start..cursor).unwrap_or_default();
        if frame
            .split(|byte| matches!(byte, b'\n' | b'\r'))
            .any(|line| line.starts_with(b"data:"))
        {
            return complete_frames
                .split_at_checked(start)
                .unwrap_or((complete_frames, &[]));
        }
        start = cursor;
    }
    (complete_frames, &[])
}

pub struct Coalescer<'stream> {
    pub input: &'stream mut Receiver<Result<StreamEvent, ModelStreamError>>,
    pub pending: Vec<u8>,
    pub searching: bool,
    pub ready: VecDeque<Result<Coalesced, ModelStreamError>>,
}

impl Coalescer<'_> {
    // Relays leading data-less frames, then joins raw chunks until one frame holds a data line, so the stream shape is decided on a whole frame
    pub async fn coalesce_first_sse_frame(
        &mut self,
    ) -> Option<Result<Coalesced, ModelStreamError>> {
        loop {
            if let Some(item) = self.ready.pop_front() {
                return Some(item);
            }
            let Some(received) = self.input.recv().await else {
                self.end_search();
                return self.ready.pop_front();
            };
            if !self.searching {
                return Some(received.map(Coalesced::Event));
            }
            match received {
                Ok(StreamEvent::Bytes(chunk)) => self.join_chunk(&chunk),
                Ok(other) => return Some(Ok(Coalesced::Event(other))),
                Err(error) => {
                    self.end_search();
                    self.ready.push_back(Err(error));
                }
            }
        }
    }

    // Stops looking for the first data frame and hands out what is pending as one event
    fn end_search(&mut self) {
        self.searching = false;
        if !self.pending.is_empty() {
            let pending = mem::take(&mut self.pending);
            self.ready
                .push_back(Ok(Coalesced::Event(StreamEvent::Bytes(pending))));
        }
    }

    fn join_chunk(&mut self, chunk: &[u8]) {
        self.pending.extend_from_slice(chunk);
        let pending = mem::take(&mut self.pending);
        let (complete_frames, tail) = split_complete_sse_frames(&pending);
        let (preface, classifiable) = split_sse_preface(complete_frames);
        if !preface.is_empty() {
            self.ready
                .push_back(Ok(Coalesced::Preface(preface.to_vec())));
        }
        self.pending = [classifiable, tail].concat();
        if !classifiable.is_empty() || self.pending.len() >= MAX_FIRST_SSE_FRAME_BYTES {
            self.end_search();
        }
    }
}

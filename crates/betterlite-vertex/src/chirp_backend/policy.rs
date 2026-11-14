use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_core::api::realtime_contracts::close::CloseFrame;

use crate::chirp_stream::limits::{CLOSE_REASON_MAX_CHARS, STREAM_FAILURE_CLOSE_CODE};

mod tests;

pub const NORMAL_CLOSURE_CODE: u16 = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deliverable<'item> {
    Event(&'item str),
    TurnResult { turn: usize, event: &'item str },
    DiscardMarker { turn: usize, event: &'item str },
    StreamFailure { reason: &'item str },
    Closed,
}

#[must_use]
pub fn rotation_due(
    elapsed: Duration,
    speech_active: bool,
    rotation_seconds: f64,
    deadline_seconds: f64,
) -> bool {
    let seconds = elapsed.as_secs_f64();
    seconds >= deadline_seconds || (seconds >= rotation_seconds && !speech_active)
}

pub fn deliver(
    item: Deliverable<'_>,
    discarded: &mut BTreeSet<usize>,
) -> Result<Option<String>, CloseFrame> {
    match item {
        Deliverable::Event(event) => Ok(Some(event.to_owned())),
        Deliverable::TurnResult { turn, event } => {
            if discarded.contains(&turn) {
                Ok(None)
            } else {
                Ok(Some(event.to_owned()))
            }
        }
        Deliverable::DiscardMarker { turn, event } => {
            let _removed = discarded.remove(&turn);
            Ok(Some(event.to_owned()))
        }
        Deliverable::StreamFailure { reason } => Err(CloseFrame {
            code: STREAM_FAILURE_CLOSE_CODE,
            reason: reason.chars().take(CLOSE_REASON_MAX_CHARS).collect(),
        }),
        Deliverable::Closed => Err(CloseFrame {
            code: NORMAL_CLOSURE_CODE,
            reason: String::new(),
        }),
    }
}

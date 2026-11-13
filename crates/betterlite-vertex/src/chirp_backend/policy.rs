use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_core::api::realtime_contracts::close::CloseFrame;

use crate::chirp_stream::limits::{
    CLOSE_REASON_MAX_CHARS, STREAM_DEADLINE_SECONDS, STREAM_FAILURE_CLOSE_CODE,
    STREAM_ROTATION_SECONDS,
};

mod tests;

pub const NORMAL_CLOSURE_CODE: u16 = 1000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotationLimits {
    pub rotation_seconds: f64,
    pub deadline_seconds: f64,
}

impl Default for RotationLimits {
    fn default() -> Self {
        Self {
            rotation_seconds: STREAM_ROTATION_SECONDS,
            deadline_seconds: STREAM_DEADLINE_SECONDS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deliverable<'item> {
    Event(&'item str),
    TurnResult { turn: usize, event: &'item str },
    DiscardMarker { turn: usize, event: &'item str },
    StreamFailure { reason: &'item str },
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    Delivered(String),
    Suppressed,
    Close(CloseFrame),
}

#[must_use]
pub fn rotation_due(elapsed: Duration, speech_active: bool, limits: RotationLimits) -> bool {
    let seconds = elapsed.as_secs_f64();
    seconds >= limits.deadline_seconds || (seconds >= limits.rotation_seconds && !speech_active)
}

#[must_use]
pub fn deliver(item: Deliverable<'_>, discarded: &mut BTreeSet<usize>) -> Delivery {
    match item {
        Deliverable::Event(event) => Delivery::Delivered(event.to_owned()),
        Deliverable::TurnResult { turn, event } => {
            if discarded.contains(&turn) {
                Delivery::Suppressed
            } else {
                Delivery::Delivered(event.to_owned())
            }
        }
        Deliverable::DiscardMarker { turn, event } => {
            let _removed = discarded.remove(&turn);
            Delivery::Delivered(event.to_owned())
        }
        Deliverable::StreamFailure { reason } => Delivery::Close(CloseFrame {
            code: STREAM_FAILURE_CLOSE_CODE,
            reason: reason.chars().take(CLOSE_REASON_MAX_CHARS).collect(),
        }),
        Deliverable::Closed => Delivery::Close(CloseFrame {
            code: NORMAL_CLOSURE_CODE,
            reason: String::new(),
        }),
    }
}

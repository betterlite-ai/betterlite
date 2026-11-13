use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_vertex::chirp_backend::policy::{Deliverable, deliver, rotation_due};
use betterlite_vertex::chirp_stream::limits::{STREAM_DEADLINE_SECONDS, STREAM_ROTATION_SECONDS};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Deadline,
    Deliver,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ItemKind {
    Event,
    TurnResult,
    DiscardMarker,
    StreamFailure,
    Closed,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    kind: Kind,
    elapsed_seconds: f64,
    speech_active: bool,
    item_kind: ItemKind,
    turn: usize,
    event: String,
    reason: String,
    discarded_turns: Vec<usize>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.kind {
            Kind::Deadline => {
                let elapsed = Duration::try_from_secs_f64(input.elapsed_seconds)
                    .map_err(|_source| failure("elapsed_seconds is out of range"))?;
                let expired = rotation_due(
                    elapsed,
                    input.speech_active,
                    STREAM_ROTATION_SECONDS,
                    STREAM_DEADLINE_SECONDS,
                );
                Ok(json!({ "expired": expired }))
            }
            Kind::Deliver => {
                let deliverable = match input.item_kind {
                    ItemKind::Event => Deliverable::Event(&input.event),
                    ItemKind::TurnResult => Deliverable::TurnResult {
                        turn: input.turn,
                        event: &input.event,
                    },
                    ItemKind::DiscardMarker => Deliverable::DiscardMarker {
                        turn: input.turn,
                        event: &input.event,
                    },
                    ItemKind::StreamFailure => Deliverable::StreamFailure {
                        reason: &input.reason,
                    },
                    ItemKind::Closed => Deliverable::Closed,
                };
                let mut discarded: BTreeSet<usize> =
                    input.discarded_turns.iter().copied().collect();
                let outcome = match deliver(deliverable, &mut discarded) {
                    Ok(Some(event)) => json!({ "kind": "delivered", "event": event }),
                    Ok(None) => json!({ "kind": "suppressed" }),
                    Err(frame) => {
                        json!({ "kind": "closed", "code": frame.code, "reason": frame.reason })
                    }
                };
                Ok(json!({
                    "outcome": outcome,
                    "discarded_turns": discarded.into_iter().collect::<Vec<_>>(),
                }))
            }
        }
    }
}

type Failure = Box<HelperError>;

fn failure(message: &str) -> Failure {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

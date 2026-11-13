#![cfg(test)]

use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_core::api::realtime_contracts::close::CloseFrame;

use super::{Deliverable, NORMAL_CLOSURE_CODE, deliver, rotation_due};
use crate::chirp_stream::limits::{STREAM_DEADLINE_SECONDS, STREAM_ROTATION_SECONDS};

#[test]
fn a_stream_below_the_soft_threshold_is_reused() {
    assert!(!rotation_due(
        Duration::from_millis(239_999),
        false,
        STREAM_ROTATION_SECONDS,
        STREAM_DEADLINE_SECONDS,
    ));
}

#[test]
fn an_inactive_stream_rotates_at_the_soft_threshold() {
    assert!(rotation_due(
        Duration::from_secs(240),
        false,
        STREAM_ROTATION_SECONDS,
        STREAM_DEADLINE_SECONDS,
    ));
}

#[test]
fn speech_blocks_the_soft_threshold() {
    assert!(!rotation_due(
        Duration::from_secs(240),
        true,
        STREAM_ROTATION_SECONDS,
        STREAM_DEADLINE_SECONDS,
    ));
    assert!(!rotation_due(
        Duration::from_secs(279),
        true,
        STREAM_ROTATION_SECONDS,
        STREAM_DEADLINE_SECONDS,
    ));
}

#[test]
fn the_hard_deadline_rotates_a_speaking_stream() {
    assert!(rotation_due(
        Duration::from_secs(280),
        true,
        STREAM_ROTATION_SECONDS,
        STREAM_DEADLINE_SECONDS,
    ));
}

#[test]
fn the_hard_deadline_rotates_an_idle_stream() {
    assert!(rotation_due(
        Duration::from_secs(280),
        false,
        STREAM_ROTATION_SECONDS,
        STREAM_DEADLINE_SECONDS,
    ));
}

#[test]
fn an_event_is_delivered_as_is() {
    let mut discarded = BTreeSet::new();
    assert_eq!(
        deliver(Deliverable::Event("event"), &mut discarded),
        Ok(Some("event".to_owned()))
    );
}

#[test]
fn a_discarded_turns_result_is_hidden() {
    let mut discarded = BTreeSet::from([1]);
    assert_eq!(
        deliver(
            Deliverable::TurnResult {
                turn: 1,
                event: "result",
            },
            &mut discarded,
        ),
        Ok(None)
    );
    assert_eq!(
        deliver(
            Deliverable::TurnResult {
                turn: 2,
                event: "result",
            },
            &mut discarded,
        ),
        Ok(Some("result".to_owned()))
    );
}

#[test]
fn the_discard_marker_clears_the_turn_and_is_delivered() {
    let mut discarded = BTreeSet::from([1]);
    assert_eq!(
        deliver(
            Deliverable::DiscardMarker {
                turn: 1,
                event: "marker",
            },
            &mut discarded,
        ),
        Ok(Some("marker".to_owned()))
    );
    assert!(discarded.is_empty());
    assert_eq!(
        deliver(
            Deliverable::TurnResult {
                turn: 1,
                event: "result",
            },
            &mut discarded,
        ),
        Ok(Some("result".to_owned()))
    );
}

#[test]
fn the_closed_sentinel_closes_normally() {
    let mut discarded = BTreeSet::new();
    assert_eq!(
        deliver(Deliverable::Closed, &mut discarded),
        Err(CloseFrame {
            code: NORMAL_CLOSURE_CODE,
            reason: String::new(),
        })
    );
}

#[test]
fn a_short_failure_reason_is_kept_whole() {
    let mut discarded = BTreeSet::new();
    let reason = "the stream failed";
    assert_eq!(
        deliver(Deliverable::StreamFailure { reason }, &mut discarded,),
        Err(CloseFrame {
            code: 1011,
            reason: reason.to_owned(),
        })
    );
}

#[test]
fn a_long_failure_reason_is_cut_at_the_first_hundred_and_twenty_characters() {
    let mut discarded = BTreeSet::new();
    let reason = "é".repeat(130);
    match deliver(
        Deliverable::StreamFailure { reason: &reason },
        &mut discarded,
    ) {
        Err(frame) => {
            assert_eq!(frame.code, 1011);
            assert_eq!(frame.reason.chars().count(), 120);
            assert_eq!(frame.reason, "é".repeat(120));
        }
        other => panic!("expected a close, got {other:?}"),
    }
}

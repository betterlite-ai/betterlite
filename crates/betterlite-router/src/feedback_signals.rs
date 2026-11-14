pub mod detector;
pub mod error;
pub mod signal;
pub mod state;
pub mod turn;

mod tests;

use crate::adaptive_classification::constants::MIN_TURNS_FOR_CLEAN_CREDIT;

use self::detector::{Detector, detect_response_signals, detect_user_feedback};
use self::signal::{SignalDelta, apply_signal_delta, merge_signal_deltas};
use self::state::{SessionState, advance_session_state};
use self::turn::Turn;

#[must_use]
pub fn apply_turn(detector: &Detector, state: &mut SessionState, turn: &Turn) -> SignalDelta {
    let allow_satisfaction = !state.clean_credit_awarded
        && state.turn_count.saturating_add(1) >= i64::from(MIN_TURNS_FOR_CLEAN_CREDIT);
    let feedback = detect_user_feedback(
        detector,
        state.last_user_content.as_deref(),
        turn,
        allow_satisfaction,
    );
    let response = detect_response_signals(
        detector,
        state.last_assistant_content.as_deref(),
        &state.tool_call_history,
        turn,
    );
    let delta = merge_signal_deltas(&[feedback, response]);
    apply_signal_delta(state, delta);
    if delta.satisfaction != 0 {
        state.clean_credit_awarded = true;
    }
    advance_session_state(state, turn);
    delta
}

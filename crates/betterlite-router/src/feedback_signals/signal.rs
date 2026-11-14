use serde::{Deserialize, Serialize};

use crate::feedback_signals::state::SessionState;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalDelta {
    pub misalignment: i64,
    pub stagnation: i64,
    pub disengagement: i64,
    pub satisfaction: i64,
    pub failure: i64,
    pub r#loop: i64,
    pub exhaustion: i64,
}

impl SignalDelta {
    #[must_use]
    pub fn any_fired(&self) -> bool {
        [
            self.misalignment,
            self.stagnation,
            self.disengagement,
            self.satisfaction,
            self.failure,
            self.r#loop,
            self.exhaustion,
        ]
        .into_iter()
        .any(|count| count != 0)
    }
}

#[must_use]
pub fn merge_signal_deltas(deltas: &[SignalDelta]) -> SignalDelta {
    let mut merged = SignalDelta::default();
    for delta in deltas {
        merged.misalignment = merged.misalignment.saturating_add(delta.misalignment);
        merged.stagnation = merged.stagnation.saturating_add(delta.stagnation);
        merged.disengagement = merged.disengagement.saturating_add(delta.disengagement);
        merged.satisfaction = merged.satisfaction.saturating_add(delta.satisfaction);
        merged.failure = merged.failure.saturating_add(delta.failure);
        merged.r#loop = merged.r#loop.saturating_add(delta.r#loop);
        merged.exhaustion = merged.exhaustion.saturating_add(delta.exhaustion);
    }
    merged
}

pub const fn apply_signal_delta(state: &mut SessionState, delta: SignalDelta) {
    state.misalignment_count = state.misalignment_count.saturating_add(delta.misalignment);
    state.stagnation_count = state.stagnation_count.saturating_add(delta.stagnation);
    state.disengagement_count = state
        .disengagement_count
        .saturating_add(delta.disengagement);
    state.satisfaction_count = state.satisfaction_count.saturating_add(delta.satisfaction);
    state.failure_count = state.failure_count.saturating_add(delta.failure);
    state.loop_count = state.loop_count.saturating_add(delta.r#loop);
    state.exhaustion_count = state.exhaustion_count.saturating_add(delta.exhaustion);
}

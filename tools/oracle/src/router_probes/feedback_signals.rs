use betterlite_router::adaptive_classification::request_type::RequestType;
use betterlite_router::feedback_signals::apply_turn;
use betterlite_router::feedback_signals::detector::{Detector, DetectorThresholds};
use betterlite_router::feedback_signals::signal::SignalDelta;
use betterlite_router::feedback_signals::state::SessionState;
use betterlite_router::feedback_signals::turn::Turn;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    pub turns: Vec<Turn>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub deltas: Vec<SignalDelta>,
    pub state: StateOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateOutput {
    pub misalignment_count: i64,
    pub stagnation_count: i64,
    pub disengagement_count: i64,
    pub satisfaction_count: i64,
    pub failure_count: i64,
    pub loop_count: i64,
    pub exhaustion_count: i64,
    pub tool_call_history: Vec<String>,
    pub turn_count: i64,
    pub last_processed_turn: i64,
    pub clean_credit_awarded: bool,
    pub terminal_status: Option<i64>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let detector = Detector::new(DetectorThresholds::PINNED).map_err(|source| {
        OperationError::InvalidRequest {
            message: source.to_string(),
        }
    })?;
    let mut state = SessionState::new(
        "probe".to_owned(),
        "probe".to_owned(),
        "probe".to_owned(),
        RequestType::General,
    );
    let mut deltas = Vec::with_capacity(input.turns.len());
    for turn in &input.turns {
        deltas.push(apply_turn(&detector, &mut state, turn));
    }
    Ok(ProbeOutput::FeedbackSignals(Output {
        deltas,
        state: StateOutput::from(&state),
    }))
}

impl From<&SessionState> for StateOutput {
    fn from(state: &SessionState) -> Self {
        Self {
            misalignment_count: state.misalignment_count,
            stagnation_count: state.stagnation_count,
            disengagement_count: state.disengagement_count,
            satisfaction_count: state.satisfaction_count,
            failure_count: state.failure_count,
            loop_count: state.loop_count,
            exhaustion_count: state.exhaustion_count,
            tool_call_history: state.tool_call_history.clone(),
            turn_count: state.turn_count,
            last_processed_turn: state.last_processed_turn,
            clean_credit_awarded: state.clean_credit_awarded,
            terminal_status: state.terminal_status,
        }
    }
}

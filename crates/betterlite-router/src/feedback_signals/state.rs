use indexmap::IndexMap;

use crate::adaptive_classification::constants::TOOL_CALL_HISTORY_MAX;
use crate::adaptive_classification::request_type::RequestType;
use crate::feedback_signals::turn::Turn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    pub session_id: String,
    pub router_name: String,
    pub model_name: String,
    pub classified_type: RequestType,
    pub misalignment_count: i64,
    pub stagnation_count: i64,
    pub disengagement_count: i64,
    pub satisfaction_count: i64,
    pub failure_count: i64,
    pub loop_count: i64,
    pub exhaustion_count: i64,
    pub last_user_content: Option<String>,
    pub last_assistant_content: Option<String>,
    pub tool_call_history: Vec<String>,
    pub pending_tool_calls: IndexMap<String, String>,
    pub turn_count: i64,
    pub last_processed_turn: i64,
    pub clean_credit_awarded: bool,
    pub terminal_status: Option<i64>,
}

impl SessionState {
    #[must_use]
    pub fn new(
        session_id: String,
        router_name: String,
        model_name: String,
        classified_type: RequestType,
    ) -> Self {
        Self {
            session_id,
            router_name,
            model_name,
            classified_type,
            misalignment_count: 0,
            stagnation_count: 0,
            disengagement_count: 0,
            satisfaction_count: 0,
            failure_count: 0,
            loop_count: 0,
            exhaustion_count: 0,
            last_user_content: None,
            last_assistant_content: None,
            tool_call_history: Vec::new(),
            pending_tool_calls: IndexMap::new(),
            turn_count: 0,
            last_processed_turn: -1,
            clean_credit_awarded: false,
            terminal_status: None,
        }
    }
}

pub fn advance_session_state(state: &mut SessionState, turn: &Turn) {
    if let Some(content) = turn
        .user_content
        .as_deref()
        .filter(|content| !content.is_empty())
    {
        state.last_user_content = Some(content.to_owned());
    }
    if let Some(content) = turn
        .assistant_content
        .as_deref()
        .filter(|content| !content.is_empty())
    {
        state.last_assistant_content = Some(content.to_owned());
    }
    for call in &turn.tool_calls {
        state.tool_call_history.push(format!(
            "{}({})",
            call.function.name, call.function.arguments
        ));
    }
    let overflow = state
        .tool_call_history
        .len()
        .saturating_sub(TOOL_CALL_HISTORY_MAX);
    state.tool_call_history.drain(..overflow);
    if turn.response_status.is_some() {
        state.terminal_status = turn.response_status;
    }
    state.turn_count = state.turn_count.saturating_add(1);
    state.last_processed_turn = state.turn_count;
}

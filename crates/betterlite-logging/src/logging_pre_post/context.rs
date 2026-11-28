use super::env::PrePostEnv;
use crate::logging_state::LoggingState;

// The logging state of one provider call, beside the environment its records go through
pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

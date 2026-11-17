use betterlite_error::rate_limit::InvalidRateLimitHeader;
use betterlite_error::rate_limit::ProxyRateLimitError;

#[derive(Debug, thiserror::Error)]
pub enum SessionBudgetError {
    #[error("{0}")]
    Exceeded(Box<ProxyRateLimitError>),
    #[error("agent {agent_id} has a max_budget_per_session that is not valid JSON")]
    BudgetJson {
        agent_id: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("agent {agent_id} has a max_budget_per_session that is not a number")]
    InvalidBudget { agent_id: String },
    // a rejected request whose error could not be built: no headers are sent today, so this stays unreachable
    #[error(transparent)]
    Rejection(#[from] InvalidRateLimitHeader),
}

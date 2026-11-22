use thiserror::Error;

// status.HTTP_503_SERVICE_UNAVAILABLE, the status _raise_budget_unverifiable sends
pub const BUDGET_UNVERIFIABLE_STATUS: u16 = 503;

// LiteLLM's HTTPException 503 with detail {"error": message}; the edge renders it
#[derive(Debug, Error)]
#[error(
    "Budget enforcement unavailable: current spend could not be verified against Redis or the database, and fail_closed_budget_enforcement is enabled, so the request was rejected to avoid exceeding the configured budget. Retry shortly."
)]
pub struct BudgetUnverifiable {
    pub counter_key: String,
}

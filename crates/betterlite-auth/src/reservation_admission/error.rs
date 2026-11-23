use betterlite_error::error::details::BudgetExceededDetails;
use thiserror::Error;

use crate::reservation_settlement::error::SettlementError;

#[derive(Debug, Error)]
pub enum AdmissionError {
    // _raise_reservation_unavailable: fail_closed_budget_enforcement rejects the request instead of admitting it unreserved
    #[error(
        "the budget reservation for {counter_key} could not be written to the spend counter backend and fail_closed_budget_enforcement is enabled"
    )]
    Unavailable { counter_key: String },

    // _raise_counter_budget_exceeded: the over-budget policy refuses the reservation
    #[error("{}", .details.message.as_deref().unwrap_or("Budget has been exceeded!"))]
    BudgetExceeded { details: Box<BudgetExceededDetails> },

    #[error("the reservation cost is outside the decimal range")]
    Overflow,

    #[error(transparent)]
    Settlement(#[from] SettlementError),
}

impl AdmissionError {
    // the HTTP status a proxy response carries: 503 for an unavailable counter, the configured code for a rejection, none for the others
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Unavailable { .. } => Some(503),
            Self::BudgetExceeded { details } => Some(details.status_code),
            Self::Overflow | Self::Settlement(_) => None,
        }
    }
}

// _raise_reservation_unavailable: warns and builds the 503 the strict enforcement path rejects with
pub(crate) fn unavailable(counter_key: &str) -> AdmissionError {
    tracing::warn!(
        counter_key,
        "fail_closed_budget_enforcement: rejecting request because the budget reservation could not be written to the spend counter backend"
    );
    AdmissionError::Unavailable {
        counter_key: counter_key.to_owned(),
    }
}

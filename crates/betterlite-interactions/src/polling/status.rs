use crate::types::response::InteractionsApiResponse;
use crate::types::status::{InteractionStatus, POLLABLE_STATUSES, TERMINAL_STATUSES};

#[must_use]
pub fn is_terminal_status(status: Option<&str>) -> bool {
    status
        .and_then(InteractionStatus::parse)
        .is_some_and(|parsed| TERMINAL_STATUSES.contains(&parsed))
}

#[must_use]
pub fn is_pollable_status(status: Option<&str>) -> bool {
    status
        .and_then(InteractionStatus::parse)
        .is_some_and(|parsed| POLLABLE_STATUSES.contains(&parsed))
}

#[must_use]
pub fn is_pollable_interaction(response: &InteractionsApiResponse) -> bool {
    is_pollable_status(response.status.as_deref())
        && response.id.as_deref().is_some_and(|id| !id.is_empty())
}

#[must_use]
pub fn missing_usage_is_expected(response: &InteractionsApiResponse) -> bool {
    !response
        .status
        .as_deref()
        .and_then(InteractionStatus::parse)
        .is_some_and(InteractionStatus::produced_output)
}

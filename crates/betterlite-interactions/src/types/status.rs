use serde::{Deserialize, Serialize};

pub const TERMINAL_STATUSES: [InteractionStatus; 6] = [
    InteractionStatus::Completed,
    InteractionStatus::Failed,
    InteractionStatus::Cancelled,
    InteractionStatus::Incomplete,
    InteractionStatus::BudgetExceeded,
    InteractionStatus::RequiresAction,
];

pub const POLLABLE_STATUSES: [InteractionStatus; 2] =
    [InteractionStatus::InProgress, InteractionStatus::Queued];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionStatus {
    InProgress,
    RequiresAction,
    Completed,
    Failed,
    Cancelled,
    Incomplete,
    BudgetExceeded,
    Queued,
}

impl InteractionStatus {
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "in_progress" => Some(Self::InProgress),
            "requires_action" => Some(Self::RequiresAction),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "incomplete" => Some(Self::Incomplete),
            "budget_exceeded" => Some(Self::BudgetExceeded),
            "queued" => Some(Self::Queued),
            _ => None,
        }
    }

    #[must_use]
    pub const fn produced_output(self) -> bool {
        match self {
            Self::Completed | Self::RequiresAction => true,
            Self::InProgress
            | Self::Failed
            | Self::Cancelled
            | Self::Incomplete
            | Self::BudgetExceeded
            | Self::Queued => false,
        }
    }
}

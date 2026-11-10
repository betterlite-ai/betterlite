use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum BudgetConfigError {
    #[error("the entry is not an object")]
    NotAnObject,
    #[error("the entry has no budget_duration or time_period string")]
    MissingDuration,
    #[error("budget_duration {spec:?} is not a duration")]
    UnparsableDuration { spec: String },
    #[error("max_budget or budget_limit is not a number")]
    MaxBudget,
    #[error("{field} is not an integer")]
    Limit { field: &'static str },
}

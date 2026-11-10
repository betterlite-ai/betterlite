use thiserror::Error;

#[derive(Debug, Error)]
pub enum BudgetConfigError {
    #[error("the model_max_budget entry is not a JSON object")]
    NotObject,
    #[error("the model_max_budget entry holds a field of the wrong type")]
    Entry {
        #[source]
        source: serde_json::Error,
    },
    #[error("the entry has no budget_duration or time_period string")]
    MissingDuration,
    #[error("budget_duration {spec:?} is not a duration")]
    UnparsableDuration { spec: String },
    #[error("max_budget or budget_limit is not a number")]
    MaxBudget,
}

use std::num::TryFromIntError;

pub mod aggregation;
pub mod records;
pub mod region;
pub mod wire;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum NewRelicMetricsError {
    #[error("the cost sum overflowed the decimal range")]
    CostSum,
    #[error("a token count sum overflowed")]
    TokenSum,
    #[error("the remaining budget overflowed the decimal range")]
    BudgetSum,
    #[error("the window start does not fit in milliseconds")]
    WindowStart(#[source] TryFromIntError),
    #[error("the window length does not fit in milliseconds")]
    WindowLength(#[source] TryFromIntError),
}

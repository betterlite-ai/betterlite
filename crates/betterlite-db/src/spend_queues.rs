pub mod daily;
pub mod entity;
pub mod events;
mod interrupted;
mod queue;

mod tests;

// SpendUpdateQueue and DailySpendUpdateQueue of the source: bounded in-memory buffers that compact their entries before a flush aggregates them.
#[derive(Debug, thiserror::Error)]
pub enum SpendQueueError {
    #[error("the in-memory spend queue needs a capacity of at least one record")]
    ZeroCapacity,
    #[error("adding spend costs overflows the decimal range")]
    CostOverflow,
    #[error("daily spend field {field} is missing when aggregating")]
    MissingField { field: &'static str },
    #[error("daily spend field {field} does not hold a number to add")]
    InvalidAddition { field: &'static str },
    #[error("daily spend field {field} overflows its type")]
    FieldOverflow { field: &'static str },
}

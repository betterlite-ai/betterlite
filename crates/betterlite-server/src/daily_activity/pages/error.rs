use betterlite_analytics::canonical_date::InvalidDateRange;
use betterlite_db::daily_activity_contracts::identifiers::DailyActivityError;
use betterlite_db::daily_activity_repository::DailyActivityRepositoryError;
use thiserror::Error;
use tokio::task::JoinError;

#[derive(Debug, Error)]
pub enum PagesError {
    #[error("{}", .0.reason)]
    DateRange(InvalidDateRange),
    #[error(transparent)]
    Scope(#[from] DailyActivityError),
    #[error(transparent)]
    Repository(#[from] DailyActivityRepositoryError),
    #[error("the daily activity date {value} is not a date")]
    Date {
        value: String,
        #[source]
        source: chrono::ParseError,
    },
    #[error("daily activity metrics are out of range")]
    Overflow,
    #[error("the daily activity aggregation task failed")]
    Task {
        #[source]
        source: JoinError,
    },
    #[error("the daily activity aggregation task returned no result")]
    TaskMissing,
}

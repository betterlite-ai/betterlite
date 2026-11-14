use crate::postgres_store::diagnostics::DriverError;
use crate::user_repository::RowDecodeError;

#[derive(Debug, thiserror::Error)]
pub enum KeyAttributionError {
    #[error("the token digest row is not readable")]
    TokenDigest {
        #[source]
        source: DriverError,
    },
    #[error("the daily spend owner row is not readable")]
    DailyOwner {
        #[source]
        source: DriverError,
    },
    #[error("the stored user row is not readable")]
    User {
        #[source]
        source: RowDecodeError,
    },
}

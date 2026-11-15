use crate::postgres_store::diagnostics::DriverError;

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("the spend-log history row is not readable")]
    Row {
        #[source]
        source: DriverError,
    },
}

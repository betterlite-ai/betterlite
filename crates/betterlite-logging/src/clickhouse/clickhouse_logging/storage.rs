use std::error::Error;
use std::future::Future;

use crate::trace_types::SpendLogRecord;

pub trait ClickHouseStorage: Send + Sync + 'static {
    type Error: Error + Send + Sync + 'static;

    fn insert_rows(
        &self,
        table: &str,
        rows: &[SpendLogRecord],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn ensure_schema(
        &self,
        trace_retention_days: u32,
        spend_log_retention_days: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

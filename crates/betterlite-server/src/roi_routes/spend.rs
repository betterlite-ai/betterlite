use std::future::Future;
use std::sync::Arc;

use betterlite_db::postgres_store::errors::DbError;
use betterlite_roi::records::ROISpendRecord;
use chrono::NaiveDate;

use crate::roi_spend::{PostgresSpendReader, SpendReader};

use super::ports::{SpendFuture, SpendReads};

impl SpendReads for PostgresSpendReader {
    fn read_spend(&self, start: NaiveDate, end: NaiveDate) -> SpendFuture<'_> {
        Box::pin(SpendReader::read(self, start, end))
    }
}

impl SpendReader for Arc<dyn SpendReads> {
    fn read(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> impl Future<Output = Result<Vec<ROISpendRecord>, DbError>> + Send {
        self.read_spend(start, end)
    }
}

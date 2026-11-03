use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::spend::read_spend;
use betterlite_roi::records::ROISpendRecord;
use chrono::NaiveDate;
use std::future::Future;

pub trait SpendReader {
    fn read(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> impl Future<Output = Result<Vec<ROISpendRecord>, DbError>> + Send;
}

pub struct PostgresSpendReader {
    pub store: PostgresStore,
}

impl SpendReader for PostgresSpendReader {
    async fn read(&self, start: NaiveDate, end: NaiveDate) -> Result<Vec<ROISpendRecord>, DbError> {
        let client = self.store.session().await?;
        read_spend(&**client, start, end).await
    }
}

mod tests;

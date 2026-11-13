use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;
use tokio_postgres::Client;
pub type ExecuteFuture<'database, Error> =
    Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'database>>;
pub type QueryFuture<'database, Error> =
    Pin<Box<dyn Future<Output = Result<Vec<RawRow>, Error>> + Send + 'database>>;
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawRow {
    pub columns: BTreeMap<String, Option<i64>>,
}
pub trait SupportsExecuteRaw {
    type Error: StdError + Send + Sync + 'static;
    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
    ) -> ExecuteFuture<'database, Self::Error>;
}
pub trait SupportsRawQueries: SupportsExecuteRaw {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
    ) -> QueryFuture<'database, Self::Error>;
}
impl SupportsExecuteRaw for Client {
    type Error = tokio_postgres::Error;
    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
    ) -> ExecuteFuture<'database, Self::Error> {
        Box::pin(async move { self.batch_execute(query).await })
    }
}
impl SupportsRawQueries for Client {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
    ) -> QueryFuture<'database, Self::Error> {
        Box::pin(async move {
            let records = self.query(query, &[]).await?;
            let rows = records
                .iter()
                .map(|row| RawRow {
                    // the probe's SELECT 1 yields int4 and no caller reads its rows; only BIGINT estimates carry a value
                    columns: row
                        .columns()
                        .iter()
                        .enumerate()
                        .filter_map(|(index, column)| {
                            let value = row.try_get::<_, Option<i64>>(index).ok()?;
                            Some((column.name().to_owned(), value))
                        })
                        .collect(),
                })
                .collect();
            Ok(rows)
        })
    }
}

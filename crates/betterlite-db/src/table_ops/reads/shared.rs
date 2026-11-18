use tokio_postgres::types::ToSql;

use crate::database_replica::client::DatabaseClient;

use crate::table_ops::access::TablePoolAccess;
use crate::table_ops::rows::RowError;
use crate::table_ops::{OperationCallType, RetryPolicy, TableOps, TableOpsError};

// every read branch is one retried query plus its own row decoder
impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub(super) async fn query_rows<Row, Parse>(
        &self,
        attribute: &str,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
        policy: RetryPolicy,
        call_type: OperationCallType,
        parse: Parse,
    ) -> Result<Vec<Row>, TableOpsError<Client::Error>>
    where
        Row: Send,
        Parse: Fn(&str) -> Result<Row, RowError> + Send + Sync,
    {
        self.attempt_with(policy, call_type, || async {
            let rows = self.text_rows(attribute, method, sql, parameters).await?;
            rows.iter()
                .map(|text| parse(text).map_err(Into::into))
                .collect()
        })
        .await
    }
}

use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use tokio_postgres::types::ToSql;

use crate::daily_activity_contracts::SqlParameter;
use crate::gateway_requests::GatewayRequestSnapshot;

use super::gateway_requests_upsert::build_gateway_requests_upsert;

mod tests;

pub type CommitFuture<'database, Value, Failure> =
    Pin<Box<dyn Future<Output = Result<Value, Failure>> + Send + 'database>>;

pub trait GatewayRequestsDatabase: Sync {
    type Error: StdError + Send + Sync + 'static;

    fn execute_upsert<'call>(
        &'call self,
        sql: &'call str,
        parameters: Vec<&'call (dyn ToSql + Sync)>,
    ) -> CommitFuture<'call, u64, Self::Error>;
}

pub struct PostgresGatewayRequests<Client> {
    pub client: Client,
}

impl<Client> GatewayRequestsDatabase for PostgresGatewayRequests<Client>
where
    Client: deadpool_postgres::GenericClient + Send + Sync,
{
    type Error = tokio_postgres::Error;

    fn execute_upsert<'call>(
        &'call self,
        sql: &'call str,
        parameters: Vec<&'call (dyn ToSql + Sync)>,
    ) -> CommitFuture<'call, u64, Self::Error> {
        Box::pin(async move { self.client.execute_raw(sql, parameters).await })
    }
}

pub async fn commit_gateway_requests_to_db<Database>(
    database: &Database,
    snapshot: &GatewayRequestSnapshot,
) -> Result<usize, Database::Error>
where
    Database: GatewayRequestsDatabase,
{
    if snapshot.is_empty() {
        return Ok(0);
    }
    let (sql, parameters) = build_gateway_requests_upsert(snapshot);
    let bound: Vec<&(dyn ToSql + Sync)> = parameters.iter().map(parameter_bind).collect();
    let _committed = database.execute_upsert(&sql, bound).await?;
    tracing::debug!(
        "Gateway request tracking - committed {} aggregated rows in one statement",
        snapshot.len()
    );
    Ok(snapshot.len())
}

fn parameter_bind(parameter: &SqlParameter) -> &(dyn ToSql + Sync) {
    match parameter {
        SqlParameter::Text(value) => value,
        SqlParameter::Texts(values) => values,
        SqlParameter::Integer(value) => value,
    }
}

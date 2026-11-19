use async_trait::async_trait;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use tokio_postgres::Row;

const AGGREGATE_SQL: &str = "SELECT date, category, route, SUM(successful_requests)::bigint AS successful_requests, SUM(failed_requests)::bigint AS failed_requests FROM \"LiteLLM_DailyGatewayRequests\" WHERE date >= $1 AND date <= $2 GROUP BY date, category, route";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GatewayActivityRow {
    pub date: String,
    pub category: String,
    pub route: String,
    pub successful_requests: i64,
    pub failed_requests: i64,
}

#[async_trait]
pub trait GatewayActivityStore: Send + Sync {
    async fn aggregate(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<GatewayActivityRow>, DbError>;
}

pub struct PostgresActivityStore {
    pub store: PostgresStore,
}

#[async_trait]
impl GatewayActivityStore for PostgresActivityStore {
    async fn aggregate(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<GatewayActivityRow>, DbError> {
        let client = self.store.session().await?;
        let rows = client
            .query(AGGREGATE_SQL, &[&start_date, &end_date])
            .await
            .map_err(backend)?;
        rows.iter().map(decode_row).collect()
    }
}

fn decode_row(row: &Row) -> Result<GatewayActivityRow, DbError> {
    Ok(GatewayActivityRow {
        date: row.try_get("date").map_err(backend)?,
        category: row.try_get("category").map_err(backend)?,
        route: row.try_get("route").map_err(backend)?,
        successful_requests: row.try_get("successful_requests").map_err(backend)?,
        failed_requests: row.try_get("failed_requests").map_err(backend)?,
    })
}

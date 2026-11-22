use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::sql::{
    EXCEPTIONS_SQL, LATENCY_SQL, SLOW_RESPONSES_SQL, STREAMING_DAILY_SQL, STREAMING_SAME_DAY_SQL,
};

type Parameter<'parameter> = &'parameter (dyn ToSql + Sync);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamingWindow {
    pub model_group: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SpendFilters {
    pub model_group: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub api_key: String,
    pub customer: String,
}

impl fmt::Debug for SpendFilters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpendFilters")
            .field("model_group", &self.model_group)
            .field("start", &self.start)
            .field("end", &self.end)
            .field("api_key", &"***")
            .field("customer", &self.customer)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlowRequest {
    pub filters: SpendFilters,
    pub threshold_seconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionWindow {
    pub model_group: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeriesValue {
    pub key: String,
    pub api_base: Option<String>,
    pub model: String,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlowRow {
    pub api_base: Option<String>,
    pub total_count: i64,
    pub slow_count: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionRow {
    pub model: String,
    pub exception_type: String,
    pub count: i64,
}

trait SqlParameters {
    fn parameters(&self) -> Vec<Parameter<'_>>;
}

impl SqlParameters for StreamingWindow {
    fn parameters(&self) -> Vec<Parameter<'_>> {
        vec![&self.model_group, &self.start, &self.end]
    }
}

impl SqlParameters for SpendFilters {
    fn parameters(&self) -> Vec<Parameter<'_>> {
        vec![
            &self.model_group,
            &self.start,
            &self.end,
            &self.api_key,
            &self.customer,
        ]
    }
}

impl SqlParameters for SlowRequest {
    fn parameters(&self) -> Vec<Parameter<'_>> {
        let mut parameters: Vec<Parameter<'_>> = vec![&self.threshold_seconds];
        parameters.extend(self.filters.parameters());
        parameters
    }
}

impl SqlParameters for ExceptionWindow {
    fn parameters(&self) -> Vec<Parameter<'_>> {
        vec![&self.start, &self.end, &self.model_group]
    }
}

#[async_trait]
pub trait ModelMetricsStore: Send + Sync {
    async fn streaming_same_day(
        &self,
        request: &StreamingWindow,
    ) -> Result<Vec<SeriesValue>, DbError>;

    async fn streaming_daily(&self, request: &StreamingWindow)
    -> Result<Vec<SeriesValue>, DbError>;

    async fn latency(&self, request: &SpendFilters) -> Result<Vec<SeriesValue>, DbError>;

    async fn slow_responses(&self, request: &SlowRequest) -> Result<Vec<SlowRow>, DbError>;

    async fn exceptions(&self, request: &ExceptionWindow) -> Result<Vec<ExceptionRow>, DbError>;
}

pub struct PostgresModelMetricsStore {
    pub store: Arc<PostgresStore>,
}

impl PostgresModelMetricsStore {
    async fn rows<Value>(
        &self,
        sql: &str,
        parameters: &[Parameter<'_>],
        decode: fn(&Row) -> Result<Value, DbError>,
    ) -> Result<Vec<Value>, DbError> {
        let client = self.store.session().await?;
        let rows = client.query(sql, parameters).await.map_err(backend)?;
        rows.iter().map(decode).collect()
    }
}

#[async_trait]
impl ModelMetricsStore for PostgresModelMetricsStore {
    async fn streaming_same_day(
        &self,
        request: &StreamingWindow,
    ) -> Result<Vec<SeriesValue>, DbError> {
        self.rows(STREAMING_SAME_DAY_SQL, &request.parameters(), series_value)
            .await
    }

    async fn streaming_daily(
        &self,
        request: &StreamingWindow,
    ) -> Result<Vec<SeriesValue>, DbError> {
        self.rows(STREAMING_DAILY_SQL, &request.parameters(), series_value)
            .await
    }

    async fn latency(&self, request: &SpendFilters) -> Result<Vec<SeriesValue>, DbError> {
        self.rows(LATENCY_SQL, &request.parameters(), series_value)
            .await
    }

    async fn slow_responses(&self, request: &SlowRequest) -> Result<Vec<SlowRow>, DbError> {
        self.rows(SLOW_RESPONSES_SQL, &request.parameters(), slow_row)
            .await
    }

    async fn exceptions(&self, request: &ExceptionWindow) -> Result<Vec<ExceptionRow>, DbError> {
        self.rows(EXCEPTIONS_SQL, &request.parameters(), exception_row)
            .await
    }
}

fn series_value(row: &Row) -> Result<SeriesValue, DbError> {
    Ok(SeriesValue {
        key: row.try_get("key").map_err(backend)?,
        api_base: row.try_get("api_base").map_err(backend)?,
        model: row.try_get("model").map_err(backend)?,
        value: row.try_get("value").map_err(backend)?,
    })
}

fn slow_row(row: &Row) -> Result<SlowRow, DbError> {
    Ok(SlowRow {
        api_base: row.try_get("api_base").map_err(backend)?,
        total_count: row.try_get("total_count").map_err(backend)?,
        slow_count: row.try_get("slow_count").map_err(backend)?,
    })
}

fn exception_row(row: &Row) -> Result<ExceptionRow, DbError> {
    Ok(ExceptionRow {
        model: row.try_get("combined_model_api_base").map_err(backend)?,
        exception_type: row.try_get("exception_type").map_err(backend)?,
        count: row.try_get("num_rate_limit_exceptions").map_err(backend)?,
    })
}

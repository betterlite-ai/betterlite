use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, NaiveDate, Utc};
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatencyRequest {
    pub model_group: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub api_key: String,
    pub customer: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlowRequest {
    pub model_group: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub api_key: String,
    pub customer: String,
    pub alerting_threshold_seconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionWindow {
    pub model_group: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SameDayRow {
    pub api_base: Option<String>,
    pub model: String,
    pub request_id: String,
    pub time_to_first_token: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DailyRow {
    pub api_base: Option<String>,
    pub model: String,
    pub day: NaiveDate,
    pub time_to_first_token: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LatencyRow {
    pub api_base: Option<String>,
    pub model: String,
    pub day: NaiveDate,
    pub avg_latency_per_token: f64,
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

#[async_trait]
pub trait ModelMetricsStore: Send + Sync {
    async fn streaming_same_day(
        &self,
        request: &StreamingWindow,
    ) -> Result<Vec<SameDayRow>, DbError>;

    async fn streaming_daily(&self, request: &StreamingWindow) -> Result<Vec<DailyRow>, DbError>;

    async fn latency(&self, request: &LatencyRequest) -> Result<Vec<LatencyRow>, DbError>;

    async fn slow_responses(&self, request: &SlowRequest) -> Result<Vec<SlowRow>, DbError>;

    async fn exceptions(&self, request: &ExceptionWindow) -> Result<Vec<ExceptionRow>, DbError>;
}

pub struct PostgresModelMetricsStore {
    pub store: Arc<PostgresStore>,
}

#[async_trait]
impl ModelMetricsStore for PostgresModelMetricsStore {
    async fn streaming_same_day(
        &self,
        request: &StreamingWindow,
    ) -> Result<Vec<SameDayRow>, DbError> {
        let client = self.store.session().await?;
        let parameters: [Parameter<'_>; 2] = [&request.model_group, &request.start];
        let rows = client
            .query(STREAMING_SAME_DAY_SQL, &parameters)
            .await
            .map_err(backend)?;
        rows.iter().map(same_day_row).collect()
    }

    async fn streaming_daily(&self, request: &StreamingWindow) -> Result<Vec<DailyRow>, DbError> {
        let client = self.store.session().await?;
        let parameters: [Parameter<'_>; 3] = [&request.model_group, &request.start, &request.end];
        let rows = client
            .query(STREAMING_DAILY_SQL, &parameters)
            .await
            .map_err(backend)?;
        rows.iter().map(daily_row).collect()
    }

    async fn latency(&self, request: &LatencyRequest) -> Result<Vec<LatencyRow>, DbError> {
        let client = self.store.session().await?;
        let parameters: [Parameter<'_>; 5] = [
            &request.model_group,
            &request.start,
            &request.end,
            &request.api_key,
            &request.customer,
        ];
        let rows = client
            .query(LATENCY_SQL, &parameters)
            .await
            .map_err(backend)?;
        rows.iter().map(latency_row).collect()
    }

    async fn slow_responses(&self, request: &SlowRequest) -> Result<Vec<SlowRow>, DbError> {
        let client = self.store.session().await?;
        let parameters: [Parameter<'_>; 6] = [
            &request.alerting_threshold_seconds,
            &request.model_group,
            &request.start,
            &request.end,
            &request.api_key,
            &request.customer,
        ];
        let rows = client
            .query(SLOW_RESPONSES_SQL, &parameters)
            .await
            .map_err(backend)?;
        rows.iter().map(slow_row).collect()
    }

    async fn exceptions(&self, request: &ExceptionWindow) -> Result<Vec<ExceptionRow>, DbError> {
        let client = self.store.session().await?;
        let parameters: [Parameter<'_>; 3] = [&request.start, &request.end, &request.model_group];
        let rows = client
            .query(EXCEPTIONS_SQL, &parameters)
            .await
            .map_err(backend)?;
        rows.iter().map(exception_row).collect()
    }
}

fn same_day_row(row: &Row) -> Result<SameDayRow, DbError> {
    Ok(SameDayRow {
        api_base: row.try_get("api_base").map_err(backend)?,
        model: row.try_get("model").map_err(backend)?,
        request_id: row.try_get("request_id").map_err(backend)?,
        time_to_first_token: row.try_get("time_to_first_token").map_err(backend)?,
    })
}

fn daily_row(row: &Row) -> Result<DailyRow, DbError> {
    Ok(DailyRow {
        api_base: row.try_get("api_base").map_err(backend)?,
        model: row.try_get("model").map_err(backend)?,
        day: row.try_get("day").map_err(backend)?,
        time_to_first_token: row.try_get("time_to_first_token").map_err(backend)?,
    })
}

fn latency_row(row: &Row) -> Result<LatencyRow, DbError> {
    Ok(LatencyRow {
        api_base: row.try_get("api_base").map_err(backend)?,
        model: row.try_get("model").map_err(backend)?,
        day: row.try_get("day").map_err(backend)?,
        avg_latency_per_token: row.try_get("avg_latency_per_token").map_err(backend)?,
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

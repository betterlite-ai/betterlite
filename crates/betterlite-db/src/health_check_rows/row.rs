use betterlite_models::decisions::Json;
use chrono::{DateTime, NaiveDateTime, Utc};
use deadpool_postgres::Client;
use tokio_postgres::Row;
use tokio_postgres::types::{FromSql, ToSql};

use super::{HealthCheckRow, HealthCheckRowsError};

fn column<'row, T>(row: &'row Row, name: &'static str) -> Result<T, HealthCheckRowsError>
where
    T: FromSql<'row>,
{
    row.try_get(name)
        .map_err(|source| HealthCheckRowsError::Column {
            column: name,
            source,
        })
}

// _assume_utc: the columns are timestamp without time zone and their values are UTC
fn timestamp(row: &Row, name: &'static str) -> Result<DateTime<Utc>, HealthCheckRowsError> {
    let naive: NaiveDateTime = column(row, name)?;
    Ok(naive.and_utc())
}

// the validator on details: a JSON text value is parsed once more, and text that is not JSON fails the row
fn details_value(row: &Row) -> Result<Option<Json>, HealthCheckRowsError> {
    let text: Option<String> = column(row, "details")?;
    let Some(text) = text else {
        return Ok(None);
    };
    let value: Json =
        serde_json::from_str(&text).map_err(|source| HealthCheckRowsError::Details { source })?;
    match value {
        Json::Text(inner) => {
            let decoded = serde_json::from_str(&inner)
                .map_err(|source| HealthCheckRowsError::Details { source })?;
            Ok(Some(decoded))
        }
        other => Ok(Some(other)),
    }
}

pub(super) fn decode(row: &Row) -> Result<HealthCheckRow, HealthCheckRowsError> {
    Ok(HealthCheckRow {
        health_check_id: column(row, "health_check_id")?,
        model_name: column(row, "model_name")?,
        model_id: column(row, "model_id")?,
        status: column(row, "status")?,
        healthy_count: column(row, "healthy_count")?,
        unhealthy_count: column(row, "unhealthy_count")?,
        error_message: column(row, "error_message")?,
        response_time_ms: column(row, "response_time_ms")?,
        details: details_value(row)?,
        checked_by: column(row, "checked_by")?,
        checked_at: timestamp(row, "checked_at")?,
        created_at: timestamp(row, "created_at")?,
        updated_at: timestamp(row, "updated_at")?,
    })
}

pub(super) async fn decode_all(
    client: &Client,
    statement: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<Vec<HealthCheckRow>, HealthCheckRowsError> {
    let rows = client
        .query(statement, parameters)
        .await
        .map_err(|source| HealthCheckRowsError::Query { source })?;
    rows.iter().map(decode).collect()
}

use betterlite_models::decisions::Json;
use chrono::NaiveDateTime;
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

// _assume_utc: the table's timestamp columns carry no zone and hold UTC
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
        checked_at: column::<NaiveDateTime>(row, "checked_at")?.and_utc(),
        created_at: column::<NaiveDateTime>(row, "created_at")?.and_utc(),
        updated_at: column::<NaiveDateTime>(row, "updated_at")?.and_utc(),
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
